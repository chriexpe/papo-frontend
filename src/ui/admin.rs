//! Perfil próprio e administração do servidor.
//!
//! São as duas telas que faltavam para o cliente cobrir o contrato: o que
//! você é (apelido, recado, presença, foto, senha, sessões) e o que o
//! servidor é (nome, visibilidade, figurinhas, auditoria).

use egui::RichText;

use crate::api::models::{UpdateServerRequest, UpdateUserRequest};
use crate::i18n::Strings;
use crate::state::{Presence, Store};

use super::theme::{self, Tokens};

/// O que a tela pediu; o `app` traduz em comandos de rede.
#[derive(Clone, Debug)]
pub enum AdminAction {
    SaveProfile(Box<UpdateUserRequest>),
    SetPresence(Option<String>),
    PickAvatar,
    ChangePassword(String),
    LoadDevices,
    DropConnection(String),
    SaveServer(Box<UpdateServerRequest>),
    PickEmoji(String),
    DeleteEmoji(String),
    LoadAuditLogs,
}

#[derive(Default)]
pub struct AdminState {
    pub profile_open: bool,
    pub server_open: bool,
    /// Os campos só são carregados do `Store` na abertura; depois disso o
    /// que vale é o que está sendo digitado.
    loaded_profile: bool,
    loaded_server: bool,
    nickname: String,
    description: String,
    status_message: String,
    password: String,
    server_name: String,
    server_public: bool,
    server_password: String,
    emoji_name: String,
}

impl AdminState {
    pub fn open_profile(&mut self) {
        self.profile_open = true;
        self.loaded_profile = false;
    }

    pub fn open_server(&mut self) {
        self.server_open = true;
        self.loaded_server = false;
    }
}

fn sheet(t: &Tokens, ctx: &egui::Context) -> egui::Frame {
    egui::Frame::new()
        .fill(t.elevated_bg)
        .corner_radius(egui::CornerRadius::same(theme::radius::SHEET))
        .inner_margin(egui::Margin::same(theme::space::XL as i8))
        .stroke(egui::Stroke::new(1.0, t.separator))
        .shadow(ctx.global_style().visuals.window_shadow)
}

fn caption(ui: &mut egui::Ui, t: &Tokens, label: &str) {
    ui.add_space(theme::space::MD);
    ui.label(
        RichText::new(label)
            .font(theme::text::caption())
            .color(t.label_tertiary),
    );
    ui.add_space(theme::space::XS);
}

// ---------------------------------------------------------------------------

pub fn profile_window(
    ctx: &egui::Context,
    state: &mut AdminState,
    store: &Store,
    t: &Tokens,
    s: &Strings,
) -> Vec<AdminAction> {
    let mut actions = Vec::new();
    if !state.profile_open {
        return actions;
    }
    if !state.loaded_profile {
        state.loaded_profile = true;
        state.nickname = store.my_name.clone();
        state.description.clear();
        state.status_message.clear();
        state.password.clear();
        actions.push(AdminAction::LoadDevices);
    }
    let mut open = state.profile_open;

    egui::Window::new(s.profile)
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(380.0)
        .frame(sheet(t, ctx))
        .show(ctx, |ui| {
            caption(ui, t, s.nickname);
            ui.add(
                egui::TextEdit::singleline(&mut state.nickname)
                    .char_limit(32)
                    .desired_width(f32::INFINITY),
            );

            caption(ui, t, s.status_message);
            ui.add(
                egui::TextEdit::singleline(&mut state.status_message)
                    .char_limit(64)
                    .desired_width(f32::INFINITY),
            );

            caption(ui, t, s.description);
            ui.add(
                egui::TextEdit::multiline(&mut state.description)
                    .char_limit(512)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );

            ui.add_space(theme::space::MD);
            if ui.button(s.save).clicked() {
                actions.push(AdminAction::SaveProfile(Box::new(UpdateUserRequest {
                    nickname: state.nickname.trim().to_owned(),
                    status: state.status_message.trim().to_owned(),
                    description: state.description.trim().to_owned(),
                    typing: None,
                })));
            }

            // A presença é um endpoint só dela: `away`, `busy` ou nada.
            caption(ui, t, s.presence);
            ui.horizontal(|ui| {
                let me = store.member(&store.me).map(|member| member.presence);
                for (label, value, presence) in [
                    (s.presence_online, None, Presence::Online),
                    (s.presence_away, Some("away"), Presence::Away),
                    (s.presence_busy, Some("busy"), Presence::Busy),
                ] {
                    let selected = me == Some(presence);
                    if ui
                        .add(egui::Button::selectable(selected, label))
                        .clicked()
                    {
                        actions.push(AdminAction::SetPresence(value.map(str::to_owned)));
                    }
                }
            });

            caption(ui, t, s.avatar);
            if ui.button(s.change_avatar).clicked() {
                actions.push(AdminAction::PickAvatar);
            }

            caption(ui, t, s.new_password);
            ui.add(
                egui::TextEdit::singleline(&mut state.password)
                    .password(true)
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(theme::space::XS);
            if ui
                .add_enabled(
                    state.password.chars().count() >= 8,
                    egui::Button::new(s.change_password),
                )
                .clicked()
            {
                actions.push(AdminAction::ChangePassword(state.password.clone()));
                state.password.clear();
            }

            // Sessões: o backend derruba todas quando vê um token reusado,
            // então saber quais existem é parte de entender o próprio login.
            caption(ui, t, s.sessions);
            for device in &store.devices {
                ui.horizontal(|ui| {
                    let when = device
                        .created_at
                        .map(|at| {
                            at.with_timezone(&chrono::Local)
                                .format("%d/%m %H:%M")
                                .to_string()
                        })
                        .unwrap_or_else(|| device.id.clone());
                    ui.label(RichText::new(when).font(theme::text::footnote()));
                    if ui.small_button(s.drop_session).clicked() {
                        actions.push(AdminAction::DropConnection(device.id.clone()));
                    }
                });
            }
            if !store.devices.is_empty() && ui.button(s.drop_all_sessions).clicked() {
                actions.push(AdminAction::DropConnection("ALL".to_owned()));
            }
        });

    state.profile_open = open;
    actions
}

// ---------------------------------------------------------------------------

pub fn server_window(
    ctx: &egui::Context,
    state: &mut AdminState,
    store: &Store,
    t: &Tokens,
    s: &Strings,
) -> Vec<AdminAction> {
    let mut actions = Vec::new();
    if !state.server_open {
        return actions;
    }
    if !state.loaded_server {
        state.loaded_server = true;
        state.server_name = store
            .server
            .as_ref()
            .map(|server| server.name.clone())
            .unwrap_or_default();
        state.server_public = true;
        state.server_password.clear();
        actions.push(AdminAction::LoadAuditLogs);
    }
    let mut open = state.server_open;

    egui::Window::new(s.server_settings)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(420.0)
        .default_height(460.0)
        .frame(sheet(t, ctx))
        .show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                caption(ui, t, s.server_name);
                ui.add(
                    egui::TextEdit::singleline(&mut state.server_name)
                        .char_limit(64)
                        .desired_width(f32::INFINITY),
                );

                ui.add_space(theme::space::MD);
                ui.checkbox(&mut state.server_public, s.server_public);
                ui.label(
                    RichText::new(s.server_public_hint)
                        .font(theme::text::footnote())
                        .color(t.label_tertiary),
                );

                // Fechar o servidor exige senha; o backend recusa sem ela.
                if !state.server_public {
                    caption(ui, t, s.server_password);
                    ui.add(
                        egui::TextEdit::singleline(&mut state.server_password)
                            .password(true)
                            .desired_width(f32::INFINITY),
                    );
                }

                ui.add_space(theme::space::MD);
                let ready = !state.server_name.trim().is_empty()
                    && (state.server_public || state.server_password.chars().count() >= 8);
                if ui.add_enabled(ready, egui::Button::new(s.save)).clicked() {
                    actions.push(AdminAction::SaveServer(Box::new(UpdateServerRequest {
                        name: state.server_name.trim().to_owned(),
                        public: Some(state.server_public),
                        password: (!state.server_password.is_empty())
                            .then(|| state.server_password.clone()),
                        ..Default::default()
                    })));
                }

                caption(ui, t, s.server_emojis);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut state.emoji_name)
                            .hint_text(s.emoji_name)
                            .char_limit(32)
                            .desired_width(140.0),
                    );
                    if ui
                        .add_enabled(
                            !state.emoji_name.trim().is_empty(),
                            egui::Button::new(s.add_emoji),
                        )
                        .clicked()
                    {
                        actions.push(AdminAction::PickEmoji(state.emoji_name.trim().to_owned()));
                        state.emoji_name.clear();
                    }
                });
                for emoji in &store.emojis {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!(":{}:", emoji.name)).font(theme::text::footnote()));
                        if ui.small_button(s.delete).clicked() {
                            actions.push(AdminAction::DeleteEmoji(emoji.id.clone()));
                        }
                    });
                }

                caption(ui, t, s.audit_log);
                for entry in store.audit_logs.iter().take(50) {
                    let when = entry
                        .created_at
                        .map(|at| {
                            at.with_timezone(&chrono::Local)
                                .format("%d/%m %H:%M")
                                .to_string()
                        })
                        .unwrap_or_default();
                    ui.label(
                        RichText::new(format!(
                            "{when} · {} · {}",
                            entry.actor_username, entry.action
                        ))
                        .font(theme::text::footnote())
                        .color(t.label_secondary),
                    );
                }
            });
        });

    state.server_open = open;
    actions
}
