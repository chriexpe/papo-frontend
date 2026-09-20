//! Tela de cargos: quem pode o quê no servidor.
//!
//! É por aqui que se libera `send_attachment` (anexo) e `manage_channels`
//! (criar canal). O dono do servidor já pode tudo sem cargo nenhum; todo o
//! resto depende de estar num cargo que permita.

use egui::{Color32, RichText};

use crate::api::models::{parse_hex_color, RolePermissions};
use crate::i18n::Strings;
use crate::state::Store;

use super::theme::{self, Tokens};

/// O que a tela pediu. O `app` traduz em comandos de rede.
#[derive(Clone, Debug)]
pub enum RoleAction {
    Create {
        name: String,
        color: Option<String>,
        permissions: RolePermissions,
    },
    Update {
        role_id: String,
        name: String,
        color: Option<String>,
        permissions: RolePermissions,
    },
    Delete(String),
    Assign {
        user_id: String,
        role_id: String,
    },
    Unassign {
        user_id: String,
        role_id: String,
    },
}

/// Rascunho do cargo em edição. Só vira pedido quando se clica em salvar,
/// para que marcar cinco permissões não vire cinco viagens ao servidor.
#[derive(Clone, Debug, Default)]
pub struct Draft {
    /// `None` enquanto o cargo ainda não existe no servidor.
    pub id: Option<String>,
    pub name: String,
    pub color: String,
    pub permissions: RolePermissions,
}

#[derive(Default)]
pub struct RolesState {
    pub open: bool,
    pub draft: Option<Draft>,
    /// Nome do cargo que acabou de ser criado. A resposta do servidor é uma
    /// lista nova, não o cargo; guardar o nome é como se descobre o id dele
    /// para já abrir a coluna de quem o tem — sem isso era preciso clicar no
    /// cargo na lista da esquerda, e parecia que atribuir não funcionava.
    awaiting: Option<String>,
}

impl RolesState {
    /// Carrega o cargo escolhido no rascunho, descartando o que estava lá.
    fn select(&mut self, role: &crate::api::models::Role) {
        self.draft = Some(Draft {
            id: Some(role.id.clone()),
            name: role.name.clone(),
            color: role.color.clone().unwrap_or_default(),
            permissions: role.permissions,
        });
    }
}

pub fn window(
    ctx: &egui::Context,
    state: &mut RolesState,
    store: &Store,
    t: &Tokens,
    s: &Strings,
) -> Vec<RoleAction> {
    let mut actions = Vec::new();
    if !state.open {
        return actions;
    }
    let mut open = state.open;

    // O cargo criado já existe na lista: abre-o, para que atribuir a alguém
    // seja o passo seguinte e não uma descoberta.
    if let Some(name) = state.awaiting.clone() {
        if let Some(role) = store.roles.iter().find(|role| role.name == name) {
            state.select(role);
            state.awaiting = None;
        }
    }

    egui::Window::new(s.roles)
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(760.0)
        .default_height(440.0)
        .frame(
            egui::Frame::new()
                .fill(t.elevated_bg)
                .corner_radius(egui::CornerRadius::same(theme::radius::SHEET))
                .inner_margin(egui::Margin::same(theme::space::XL as i8))
                .stroke(egui::Stroke::new(1.0, t.separator))
                .shadow(ctx.global_style().visuals.window_shadow),
        )
        .show(ctx, |ui| {
            ui.horizontal_top(|ui| {
                list(ui, state, store, t, s);
                ui.separator();
                editor(ui, state, t, s, &mut actions);
                ui.separator();
                assignees(ui, state, store, t, s, &mut actions);
            });
        });

    state.open = open;
    actions
}

/// Coluna da esquerda: os cargos que existem, mais o botão de criar.
fn list(ui: &mut egui::Ui, state: &mut RolesState, store: &Store, t: &Tokens, s: &Strings) {
    ui.vertical(|ui| {
        ui.set_width(170.0);
        egui::ScrollArea::vertical()
            .id_salt("lista-de-cargos")
            .max_height(360.0)
            .show(ui, |ui| {
                for role in &store.roles {
                    let selected = state
                        .draft
                        .as_ref()
                        .and_then(|draft| draft.id.as_deref())
                        == Some(role.id.as_str());
                    ui.horizontal(|ui| {
                        let swatch = role
                            .color
                            .as_deref()
                            .and_then(parse_hex_color)
                            .unwrap_or(t.label_tertiary);
                        let (rect, _) =
                            ui.allocate_exact_size(egui::Vec2::splat(10.0), egui::Sense::hover());
                        ui.painter().circle_filled(rect.center(), 5.0, swatch);
                        if ui.selectable_label(selected, &role.name).clicked() {
                            state.select(role);
                        }
                    });
                }
                if store.roles.is_empty() {
                    ui.label(
                        RichText::new(s.no_roles)
                            .font(theme::text::footnote())
                            .color(t.label_tertiary),
                    );
                }
            });

        ui.add_space(theme::space::MD);
        if ui.button(s.new_role).clicked() {
            state.draft = Some(Draft::default());
        }
    });
}

/// Coluna da direita: nome, cor, as sete permissões e quem tem o cargo.
fn editor(
    ui: &mut egui::Ui,
    state: &mut RolesState,
    t: &Tokens,
    s: &Strings,
    actions: &mut Vec<RoleAction>,
) {
    let Some(draft) = state.draft.as_mut() else {
        ui.vertical(|ui| {
            ui.set_width(320.0);
        });
        return;
    };
    // O cargo apagado some da tela, mas o rascunho só pode ser descartado
    // depois que o `draft` emprestado acima sair de cena.
    let mut discard = false;
    let mut pending: Option<String> = None;

    ui.vertical(|ui| {
        ui.set_width(320.0);
        egui::ScrollArea::vertical()
            .id_salt("editor-de-cargo")
            .max_height(360.0)
            .show(ui, |ui| {
                ui.label(
                    RichText::new(s.role_name)
                        .font(theme::text::caption())
                        .color(t.label_tertiary),
                );
                ui.add(
                    egui::TextEdit::singleline(&mut draft.name)
                        .char_limit(32)
                        .desired_width(f32::INFINITY),
                );

                ui.add_space(theme::space::MD);
                ui.label(
                    RichText::new(s.role_color)
                        .font(theme::text::caption())
                        .color(t.label_tertiary),
                );
                ui.horizontal(|ui| {
                    // O backend guarda a cor como `#RRGGBB`; o seletor do egui
                    // trabalha em bytes, então a conversão acontece aqui.
                    let mut rgb = parse_hex_color(&draft.color)
                        .unwrap_or(Color32::GRAY)
                        .to_array();
                    let mut picked = [rgb[0], rgb[1], rgb[2]];
                    if ui.color_edit_button_srgb(&mut picked).changed() {
                        rgb = [picked[0], picked[1], picked[2], 255];
                        draft.color = format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]);
                    }
                    ui.add(
                        egui::TextEdit::singleline(&mut draft.color)
                            .char_limit(7)
                            .desired_width(90.0),
                    );
                });

                ui.add_space(theme::space::LG);
                ui.label(
                    RichText::new(s.role_permissions)
                        .font(theme::text::caption())
                        .color(t.label_tertiary),
                );
                ui.add_space(theme::space::XS);
                for (label, value) in permission_fields(&mut draft.permissions, s) {
                    ui.checkbox(value, label);
                }

            });

        ui.add_space(theme::space::MD);
        ui.horizontal(|ui| {
            let ready = !draft.name.trim().is_empty();
            let color = (!draft.color.trim().is_empty()).then(|| draft.color.trim().to_owned());
            if ui
                .add_enabled(ready, egui::Button::new(s.save))
                .clicked()
            {
                actions.push(match draft.id.clone() {
                    Some(role_id) => RoleAction::Update {
                        role_id,
                        name: draft.name.trim().to_owned(),
                        color,
                        permissions: draft.permissions,
                    },
                    None => {
                        pending = Some(draft.name.trim().to_owned());
                        RoleAction::Create {
                            name: draft.name.trim().to_owned(),
                            color,
                            permissions: draft.permissions,
                        }
                    }
                });
            }
            if let Some(role_id) = draft.id.clone() {
                if ui.button(s.delete_role).clicked() {
                    actions.push(RoleAction::Delete(role_id));
                    discard = true;
                }
            }
        });
    });

    if discard {
        state.draft = None;
    }
    if pending.is_some() {
        state.awaiting = pending;
    }
}

/// Terceira coluna: quem tem o cargo aberto. Fica ao lado das permissões,
/// não embaixo delas — enterrada sob sete caixas de seleção, parecia que
/// atribuir cargo não existia.
fn assignees(
    ui: &mut egui::Ui,
    state: &RolesState,
    store: &Store,
    t: &Tokens,
    s: &Strings,
    actions: &mut Vec<RoleAction>,
) {
    ui.vertical(|ui| {
        ui.set_width(210.0);
        ui.label(
            RichText::new(s.role_members)
                .font(theme::text::caption())
                .color(t.label_tertiary),
        );
        ui.add_space(theme::space::XS);

        // Só um cargo que já existe pode ser dado a alguém.
        let Some(role_id) = state.draft.as_ref().and_then(|draft| draft.id.clone()) else {
            ui.label(
                RichText::new(s.role_members_hint)
                    .font(theme::text::footnote())
                    .color(t.label_tertiary),
            );
            return;
        };
        egui::ScrollArea::vertical()
            .id_salt("quem-tem-o-cargo")
            .max_height(340.0)
            .show(ui, |ui| {
                for member in &store.members {
                    let mut has = member.roles.iter().any(|id| id == &role_id);
                    if ui.checkbox(&mut has, &member.name).changed() {
                        actions.push(if has {
                            RoleAction::Assign {
                                user_id: member.id.clone(),
                                role_id: role_id.clone(),
                            }
                        } else {
                            RoleAction::Unassign {
                                user_id: member.id.clone(),
                                role_id: role_id.clone(),
                            }
                        });
                    }
                }
            });
    });
}

/// As sete permissões, com o rótulo de cada uma. Em um só lugar para que
/// acrescentar uma no backend não deixe a tela para trás sem avisar.
fn permission_fields<'a>(
    permissions: &'a mut RolePermissions,
    s: &Strings,
) -> [(&'static str, &'a mut bool); 7] {
    [
        (s.perm_manage_server, &mut permissions.manage_server),
        (s.perm_manage_channels, &mut permissions.manage_channels),
        (s.perm_manage_roles, &mut permissions.manage_roles),
        (s.perm_ban_members, &mut permissions.ban_members),
        (s.perm_pin_message, &mut permissions.pin_message),
        (s.perm_everyone_message, &mut permissions.everyone_message),
        (s.perm_send_attachment, &mut permissions.send_attachment),
    ]
}
