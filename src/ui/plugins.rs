//! Discovery and signed consent, scoped to the selected server's manage_apps grant.
use super::settings::SettingsAction;
use crate::{api::net::Command, i18n::Strings, state::Store};
use egui::Ui;
use serde_json::{Value, json};

#[derive(Default)]
pub struct PluginAdmin {
    catalog: Option<Vec<Value>>,
    selected: Option<Value>,
    options: Value,
    preview: Option<Value>,
    preview_options: Value,
    busy: bool,
    ticket: u64,
    error: Option<String>,
}
impl PluginAdmin {
    fn request(&mut self, action: &str, body: Value, out: &mut Vec<SettingsAction>) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        self.ticket = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.busy = true;
        self.error = None;
        let id = self
            .selected
            .as_ref()
            .and_then(|v| v["id"].as_str())
            .unwrap_or_default()
            .to_owned();
        out.push(SettingsAction::Plugin(Command::Plugin {
            ticket: self.ticket,
            id,
            action: action.into(),
            body,
        }));
    }
    pub fn receive(&mut self, ticket: u64, action: &str, result: Result<Value, String>) {
        if ticket != self.ticket {
            return;
        }
        self.busy = false;
        match result {
            Err(error) => {
                self.error = Some(error);
                self.preview = None;
            }
            Ok(value) => match action {
                "list" | "rescan" => {
                    self.catalog = Some(value["plugins"].as_array().cloned().unwrap_or_default());
                }
                "preview" => {
                    self.preview = Some(value);
                    self.preview_options = self.options.clone();
                }
                _ => {
                    self.catalog = None;
                    self.selected = None;
                    self.preview = None;
                }
            },
        }
    }
    pub fn show(&mut self, ui: &mut Ui, store: &Store, s: &Strings, out: &mut Vec<SettingsAction>) {
        if !store.can_manage_apps() {
            *self = Self::default();
            return;
        }
        ui.heading(s.plugins);
        if let Some(error) = &self.error {
            ui.colored_label(ui.visuals().error_fg_color, error);
        }
        if self.catalog.is_none() && !self.busy {
            self.request("list", json!({}), out);
        }
        if self.busy {
            ui.spinner();
        }
        ui.add_enabled_ui(!self.busy,|ui| {
            if ui.button(s.plugin_rescan).clicked(){self.request("rescan",json!({}),out);}
            for plugin in self.catalog.clone().unwrap_or_default() {
                ui.separator();
                let name=plugin["manifest"]["name"].as_str().or(plugin["id"].as_str()).unwrap_or_default();
                ui.label(name);
                if let Some(error)=plugin["error"].as_str().filter(|v|!v.is_empty()){ui.colored_label(ui.visuals().error_fg_color,error);}
                let state=plugin["installation"]["state"].as_str().unwrap_or("discovered");
                ui.label(match state {"enabled"=>s.plugin_enabled,"quarantined"=>s.plugin_quarantined,"disabled"|"removed"=>s.plugin_disabled,_=>s.plugin_discovered});
                if ui.add_enabled(plugin["error"].as_str().unwrap_or_default().is_empty(),egui::Button::new(s.plugin_select)).clicked(){
                    let installation=&plugin["installation"];
                    let mut settings=installation["settings"].as_object().cloned().unwrap_or_default();
                    for field in plugin["manifest"]["settings"].as_array().into_iter().flatten(){if let Some(key)=field["key"].as_str(){settings.entry(key).or_insert_with(||field.get("default").cloned().unwrap_or(match field["type"].as_str(){Some("boolean")=>json!(false),Some("integer")=>json!(0),_=>json!("")}));}}
                    self.options=json!({"settings":settings,"scopes":installation["explicit_scopes"].as_array().cloned().unwrap_or_default(),"max_memory_mb":installation["max_memory_mb"].as_i64().filter(|v|*v>0).or(plugin["manifest"]["runtime"]["max_memory_mb"].as_i64()).unwrap_or(16)});
                    self.selected=Some(plugin);self.preview=None;
                }
            }
            let Some(plugin)=self.selected.clone() else{return;};
            ui.separator();ui.heading(plugin["manifest"]["name"].as_str().unwrap_or_default());
            let before=self.options.clone();
            ui.label(s.plugin_settings);
            for field in plugin["manifest"]["settings"].as_array().into_iter().flatten(){
                let Some(key)=field["key"].as_str() else{continue;};
                ui.horizontal(|ui|{
                    ui.label(key);
                    let value=&mut self.options["settings"][key];
                    match field["type"].as_str(){
                        Some("boolean")=>{let mut v=value.as_bool().unwrap_or_default();ui.checkbox(&mut v,"");*value=json!(v);},
                        Some("integer")=>{let mut v=value.as_i64().unwrap_or_default();ui.add(egui::DragValue::new(&mut v));*value=json!(v);},
                        Some("channel")=>{let mut v=value.as_str().unwrap_or_default().to_owned();egui::ComboBox::from_id_salt(("plugin-setting",key)).selected_text(store.channel(&v).map(|c|c.name.as_str()).unwrap_or(s.plugin_select)).show_ui(ui,|ui|{for c in &store.channels {if matches!(c.kind,crate::state::ChannelKind::Text|crate::state::ChannelKind::Voice){ui.selectable_value(&mut v,c.id.clone(),&c.name);}}});*value=json!(v);},
                        _=>{let mut v=value.as_str().unwrap_or_default().to_owned();ui.text_edit_singleline(&mut v);*value=json!(v);},
                    }
                });
            }
            let mut memory=self.options["max_memory_mb"].as_i64().unwrap_or(16);
            ui.horizontal(|ui|{ui.label(s.plugin_memory);ui.add(egui::DragValue::new(&mut memory).range(1..=65536));});self.options["max_memory_mb"]=json!(memory);
            ui.label(s.plugin_scopes);
            let mut scopes:Vec<String>=self.options["scopes"].as_array().into_iter().flatten().filter_map(|v|v.as_str().map(str::to_owned)).collect();
            for channel in &store.channels {if matches!(channel.kind,crate::state::ChannelKind::Text|crate::state::ChannelKind::Voice){let mut checked=scopes.contains(&channel.id);if ui.checkbox(&mut checked,&channel.name).changed(){scopes.retain(|id|id!=&channel.id);if checked{scopes.push(channel.id.clone());}}}}
            self.options["scopes"]=json!(scopes);
            if self.options!=before {self.preview=None;}
            if ui.button(s.plugin_preview).clicked(){self.request("preview",self.options.clone(),out);}
            if let Some(preview)=self.preview.clone(){
                // Every displayed grant/value comes from the signed backend preview.
                for (key,label) in [("permissions",s.plugin_permissions),("blocked_permissions",s.plugin_blocked),("origins",s.plugin_origins),("scopes",s.plugin_scopes)] {
                    ui.label(label);
                    let entries=preview[key].as_array().cloned().unwrap_or_default();if entries.is_empty(){ui.label(s.plugin_none);}
                    for entry in entries {ui.label(entry.as_str().unwrap_or_default());}
                }
                ui.label(s.plugin_channels);
                for surface in preview["surfaces"].as_array().into_iter().flatten().filter(|v|v["kind"]=="channel") {ui.label(surface["default_name"].as_str().unwrap_or_default());ui.label(surface["entry"].as_str().unwrap_or_default());}
                ui.label(s.plugin_settings);
                for (key,value) in preview["settings"].as_object().into_iter().flatten(){ui.label(format!("{key}: {}",value.as_str().map(str::to_owned).unwrap_or_else(||value.to_string())));}
                ui.label(format!("{}: {}",s.plugin_memory,preview["max_memory_mb"]));
                if ui.button(s.plugin_approve).clicked(){let mut opts=self.preview_options.clone();opts["consent"]=preview["consent"].clone();opts["expires_at"]=preview["expires_at"].clone();self.request("enable",opts,out);}
            }
            if !plugin["installation"].is_null(){
                if ui.button(s.plugin_save).clicked(){self.request("patch",self.options.clone(),out);}
                if ui.button(s.plugin_disable).clicked(){self.request("disable",json!({}),out);}
            }
        });
    }
}
