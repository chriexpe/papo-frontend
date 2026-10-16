//! Native app controls and authenticated launch coordination. No credentials enter a view.
use crate::{
    api::{
        models::{AppInvite, AppSurface, LinkPreview},
        net::{Command, NetSender},
    },
    i18n::Strings,
    state::Store,
};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Surface {
    pub id: String,
    pub app: AppSurface,
    pub title: String,
    pub name: String,
    pub overlay: bool,
    pub message_id: String,
}
struct Pending {
    key: String,
    action: String,
    started: Instant,
    surface: Option<Surface>,
}
#[derive(Default)]
pub struct AppSurfaces {
    pub sender: Option<NetSender>,
    pending: HashMap<u64, Pending>,
    pub overlay: Option<Surface>,
    pub inline: Option<(Surface, [f32; 2])>,
    pub desired: Option<String>,
    pub error: Option<Instant>,
    pub error_key: Option<String>,
    visible: HashMap<String, (Instant, Instant, bool)>,
    pub invite: Option<AppInvite>,
    pub invalidated: HashSet<String>,
    context: (String, String),
}
impl AppSurfaces {
    pub fn sync(&mut self, server: &str, channel: &str) {
        if self.context != (server.into(), channel.into()) {
            if self.context.0 != server {
                self.invalidated.clear();
                self.invite = None;
            }
            self.pending.clear();
            self.overlay = None;
            self.inline = None;
            self.desired = None;
            self.visible.clear();
            self.context = (server.into(), channel.into());
        }
    }
    pub fn close(&mut self) {
        self.overlay = None;
        self.inline = None;
        self.desired = None;
        self.pending.retain(|_, p| p.surface.is_none());
    }
    pub fn invalidate_message(&mut self, message: &str) {
        let affected = self
            .overlay
            .as_ref()
            .is_some_and(|v| v.message_id == message)
            || self
                .inline
                .as_ref()
                .is_some_and(|(v, _)| v.message_id == message)
            || self
                .pending
                .values()
                .any(|p| p.surface.as_ref().is_some_and(|v| v.message_id == message));
        if affected {
            self.close();
        }
    }
    pub fn invalidate(&mut self, installation: &str) {
        if self.pending.values().any(|p| {
            p.surface
                .as_ref()
                .is_some_and(|v| v.app.app_installation_id == installation)
        }) {
            self.close();
        }
        self.invalidated.insert(installation.into());
        if self
            .inline
            .as_ref()
            .is_some_and(|(v, _)| v.app.app_installation_id == installation)
        {
            self.inline = None;
        }
        if self
            .overlay
            .as_ref()
            .is_some_and(|v| v.app.app_installation_id == installation)
        {
            self.close();
        }
        self.pending.retain(|_, p| {
            !p.surface
                .as_ref()
                .is_some_and(|v| v.app.app_installation_id == installation)
        });
    }
    pub fn surface_pending(&self, message: &str, surface: &str) -> bool {
        self.pending.values().any(|p| {
            p.surface
                .as_ref()
                .is_some_and(|v| v.message_id == message && v.app.surface_id == surface)
        })
    }
    pub fn busy(&self, key: &str) -> bool {
        self.pending.values().any(|p| p.key == key)
    }
    pub fn request(
        &mut self,
        key: String,
        id: String,
        action: &str,
        body: Value,
        surface: Option<Surface>,
    ) {
        if self.busy(&key) {
            return;
        }
        let Some(sender) = &self.sender else {
            return;
        };
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1 << 48);
        let ticket = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.pending.insert(
            ticket,
            Pending {
                key,
                action: action.into(),
                started: Instant::now(),
                surface,
            },
        );
        sender.send(Command::Plugin {
            ticket,
            id,
            action: action.into(),
            body,
        });
    }
    pub fn launch(&mut self, surface: Surface) {
        if !surface.app.available
            || self.invalidated.contains(&surface.app.app_installation_id)
            || self.desired.as_deref() == Some(&surface.id)
        {
            return;
        }
        self.pending.retain(|_, p| p.surface.is_none());
        self.inline = None;
        self.overlay = surface.overlay.then(|| surface.clone());
        self.desired = Some(surface.id.clone());
        let invite = self
            .invite
            .as_ref()
            .filter(|v| {
                v.channel_id == self.context.1
                    && v.app_installation_id == surface.app.app_installation_id
            })
            .map(|v| v.id.clone())
            .unwrap_or_default();
        self.request(format!("launch:{}", surface.id), surface.app.app_installation_id.clone(), "app:launch", json!({"surface_id":surface.app.surface_id,"channel_id":self.context.1,"message_id":surface.message_id,"invite_id":invite}), Some(surface));
    }
    pub fn visible(&mut self, id: &str, in_view: bool, ctx: &egui::Context) -> bool {
        let now = Instant::now();
        if !in_view {
            self.visible.remove(id);
            return false;
        }
        let v = self.visible.entry(id.into()).or_insert((now, now, false));
        if now.duration_since(v.1) > Duration::from_millis(200) {
            *v = (now, now, false);
        }
        v.1 = now;
        if !v.2 {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
        if !v.2 && now.duration_since(v.0) >= Duration::from_millis(300) {
            v.2 = true;
            return true;
        }
        false
    }
    pub fn observe(&mut self, id: &str, in_view: bool) {
        let now = Instant::now();
        if in_view {
            let v = self.visible.entry(id.into()).or_insert((now, now, false));
            if now.duration_since(v.1) > Duration::from_millis(200) {
                *v = (now, now, false);
            }
            v.1 = now;
        } else {
            self.visible.remove(id);
            self.pending
                .retain(|_, p| !p.surface.as_ref().is_some_and(|v| v.id == id && !v.overlay));
        }
    }
    pub fn launch_pending(&self) -> bool {
        self.pending.values().any(|p| p.surface.is_some())
    }
    pub fn release_inactive(&mut self, selected: Option<&str>) {
        if selected.is_none()
            && !self.launch_pending()
            && self.overlay.is_none()
            && self
                .desired
                .as_deref()
                .is_some_and(|v| !v.starts_with("channel-app:"))
        {
            self.desired = None;
            self.inline = None;
        }
    }
    pub fn tick(&mut self) {
        let expired = self
            .pending
            .values()
            .find(|p| p.started.elapsed() > Duration::from_secs(12))
            .map(|p| p.key.clone());
        if let Some(key) = expired {
            self.error_key = Some(key);
            self.pending
                .retain(|_, p| p.started.elapsed() <= Duration::from_secs(12));
            self.error = Some(Instant::now());
        }
    }
    pub fn receive(
        &mut self,
        ticket: u64,
        result: Result<Value, String>,
        store: &mut Store,
        view: &mut crate::channel_view::ChannelViewSurface,
        rich: &mut crate::webembed::WebEmbedManager,
        server: &str,
    ) {
        let Some(p) = self.pending.remove(&ticket) else {
            return;
        };
        match result {
            Err(_) => {
                self.error_key = Some(p.key);
                self.error = Some(Instant::now());
            }
            Ok(value) => {
                if let Some(mut surface) = p.surface {
                    let offscreen = surface.id.starts_with("inline-app:")
                        && !surface.overlay
                        && self
                            .visible
                            .get(&surface.id)
                            .is_none_or(|v| v.1.elapsed() > Duration::from_millis(250));
                    if offscreen
                        || self.context.1 != store.selected_channel
                        || self.desired.as_deref() != Some(&surface.id)
                        || self.invalidated.contains(&surface.app.app_installation_id)
                    {
                        return;
                    }
                    if let Some(ticket) = value["ticket"].as_str().filter(|t| !t.is_empty()) {
                        if let Ok(app) =
                            serde_json::from_value::<AppSurface>(value["surface"].clone())
                        {
                            if app.app_installation_id != surface.app.app_installation_id
                                || app.surface_id != surface.app.surface_id
                                || !app.available
                            {
                                self.error_key = Some(p.key.clone());
                                self.error = Some(Instant::now());
                                return;
                            }
                            surface.app = app;
                        }
                        if !surface.id.starts_with("channel-app:") {
                            surface.overlay = value["display"]
                                .as_str()
                                .map(|v| v == "overlay")
                                .unwrap_or(surface.overlay);
                            self.overlay = surface.overlay.then(|| surface.clone());
                            if surface.id.starts_with("button-app:") && !surface.overlay {
                                self.inline = Some((
                                    surface.clone(),
                                    [
                                        value["size"]["width"].as_f64().unwrap_or(480.0).max(1.0)
                                            as f32,
                                        value["size"]["height"].as_f64().unwrap_or(320.0).max(1.0)
                                            as f32,
                                    ],
                                ));
                            }
                        }
                        rich.destroy_active();
                        view.activate_ticket(surface.id, surface.app, server, ticket);
                    } else {
                        self.error_key = Some(p.key);
                        self.error = Some(Instant::now());
                    }
                } else if p.action == "app:accept" {
                    if let Ok(invite) = serde_json::from_value::<AppInvite>(value) {
                        self.close();
                        view.destroy();
                        rich.destroy_active();
                        store.selected_channel = invite.channel_id.clone();
                        store.app_invites.retain(|v| v.id != invite.id);
                        self.invite = Some(invite);
                    }
                } else if p.action == "app:decline" {
                    store
                        .app_invites
                        .retain(|v| format!("invite:{}", v.id) != p.key);
                }
            }
        }
    }
}
pub fn frame_app(p: &LinkPreview) -> Option<AppSurface> {
    if let Some(app) = &p.app {
        return Some(app.clone());
    }
    let reference = p.surface.as_ref()?;
    Some(AppSurface {
        app_installation_id: reference.app_installation_id.clone(),
        surface_id: reference.id.clone(),
        source: if p.entry.starts_with("/apps/") {
            "bundled"
        } else {
            "remote"
        }
        .into(),
        entry: p.entry.clone(),
        avatar_url: format!("/apps/{}/avatar", reference.app_installation_id),
        available: !p.entry.is_empty(),
        user_actions: vec![],
    })
}
pub fn error(ui: &mut egui::Ui, apps: &AppSurfaces, s: &Strings, prefix: &str) {
    if !apps
        .error_key
        .as_deref()
        .is_some_and(|v| v.starts_with(prefix))
    {
        return;
    }
    if apps
        .error
        .is_some_and(|v| v.elapsed() < Duration::from_secs(4))
    {
        ui.colored_label(ui.visuals().error_fg_color, s.app_no_response);
        ui.ctx().request_repaint_after(Duration::from_secs(4));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn surface(id: &str) -> Surface {
        Surface {
            id: id.into(),
            app: AppSurface {
                app_installation_id: "app".into(),
                surface_id: "home".into(),
                source: "bundled".into(),
                entry: "/apps/app/ui/index.html".into(),
                avatar_url: String::new(),
                available: true,
                user_actions: vec![],
            },
            title: String::new(),
            name: "Reference".into(),
            overlay: false,
            message_id: String::new(),
        }
    }
    #[test]
    fn visibility_debounces_and_fires_once_per_appearance() {
        let mut apps = AppSurfaces::default();
        let ctx = egui::Context::default();
        assert!(!apps.visible("a", true, &ctx));
        let now = Instant::now();
        apps.visible
            .insert("a".into(), (now - Duration::from_millis(250), now, false));
        assert!(!apps.visible("a", true, &ctx));
        apps.visible.get_mut("a").unwrap().0 = now - Duration::from_millis(301);
        assert!(apps.visible("a", true, &ctx));
        assert!(!apps.visible("a", true, &ctx));
        assert!(!apps.visible("a", false, &ctx));
        assert!(!apps.visible("a", true, &ctx));
    }
    #[test]
    fn latest_activation_cancels_old_launch_and_deduplicates_current() {
        let (sender, mut requests) = NetSender::channel();
        let mut apps = AppSurfaces {
            sender: Some(sender),
            ..Default::default()
        };
        apps.sync("http://localhost", "channel");
        apps.launch(surface("a"));
        apps.launch(surface("a"));
        apps.launch(surface("b"));
        assert_eq!(apps.pending.len(), 1);
        assert_eq!(apps.desired.as_deref(), Some("b"));
        assert!(requests.try_recv().is_ok());
        assert!(requests.try_recv().is_ok());
        assert!(requests.try_recv().is_err());
        apps.sync("http://localhost", "other");
        assert!(apps.pending.is_empty());
        assert!(apps.desired.is_none());
    }
    #[test]
    fn revocation_cancels_pending_launch_and_prevents_reactivation() {
        let (sender, _) = NetSender::channel();
        let mut apps = AppSurfaces {
            sender: Some(sender),
            ..Default::default()
        };
        apps.launch(surface("a"));
        apps.invalidate("app");
        assert!(apps.pending.is_empty());
        apps.launch(surface("b"));
        assert!(apps.pending.is_empty());
    }
    #[test]
    fn short_error_releases_pending_controls_after_timeout() {
        let (sender, _) = NetSender::channel();
        let mut apps = AppSurfaces {
            sender: Some(sender),
            ..Default::default()
        };
        apps.request(
            "button".into(),
            "app".into(),
            "app:interact",
            json!({}),
            None,
        );
        for p in apps.pending.values_mut() {
            p.started -= Duration::from_secs(13);
        }
        apps.tick();
        assert!(!apps.busy("button"));
        assert!(apps.error.is_some());
    }
    #[test]
    fn c5_frame_contract_has_no_provider_dependency() {
        let preview: LinkPreview = serde_json::from_value(json!({"id":"frame","source_type":"app","surface":{"app_installation_id":"app","id":"result"},"entry":"/apps/app/ui/result.html","size":{"width":560,"height":420},"load":"visible","display":"inline"})).unwrap();
        let app = frame_app(&preview).unwrap();
        assert_eq!(app.surface_id, "result");
        assert!(app.available);
        assert!(preview.title.is_none());
    }
}
