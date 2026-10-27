//! Webhook management follows the same list/editor pattern as channel settings.
use super::{
    settings::{self, Emphasis, SettingsAction, px, row_button, section, sheet_card},
    shell::ChatAction,
    theme::{Tokens, space, text},
    widgets,
};
use crate::{
    i18n::Strings,
    state::{ChannelKind, Store},
};
use egui::{Align, Layout, RichText, Sense, Vec2};
use egui_phosphor::regular as icon;
use papo_core::state::webhooks::WebhookAction;

#[derive(Default)]
pub struct Draft {
    creating: bool,
    name: String,
    channel: String,
    editing: Option<(String, String)>,
    revoking: Option<String>,
    submitted: Option<Submission>,
}

enum Submission {
    Create(Vec<String>),
    Identity(String, String),
    Revoke(String),
}

impl Draft {
    fn reconcile(&mut self, store: &Store) {
        let hooks = &store.webhooks;
        if hooks.loading() {
            return;
        }
        let confirmed = match &self.submitted {
            Some(Submission::Create(previous)) => hooks
                .created
                .as_ref()
                .is_some_and(|created| !previous.contains(&created.webhook.id)),
            Some(Submission::Identity(id, name)) => hooks.items.iter().any(|hook| {
                &hook.id == id && hook.display_name.as_deref().unwrap_or_default() == name
            }),
            Some(Submission::Revoke(id)) => !hooks.items.iter().any(|hook| &hook.id == id),
            None => false,
        };
        if confirmed {
            match self.submitted.take() {
                Some(Submission::Create(_)) => {
                    self.creating = false;
                    self.name.clear();
                }
                Some(Submission::Identity(..)) => self.editing = None,
                Some(Submission::Revoke(_)) => self.revoking = None,
                None => {}
            }
        } else if hooks.error.is_some() {
            // A rejected or uncertain operation keeps its draft for review.
            self.submitted = None;
        }
        if self
            .editing
            .as_ref()
            .is_some_and(|(id, _)| !hooks.items.iter().any(|hook| &hook.id == id))
        {
            self.editing = None;
        }
    }
}

fn dispatch(actions: &mut Vec<SettingsAction>, action: WebhookAction) {
    actions.push(SettingsAction::Chat(ChatAction::WebhookAdmin(action)));
}

fn footer(ui: &mut egui::Ui, t: &Tokens, ready: bool, label: &str, cancel: &str) -> (bool, bool) {
    let mut save = false;
    let mut discard = false;
    ui.add_space(space::LG);
    ui.horizontal(|ui| {
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_enabled_ui(ready, |ui| {
                save = row_button(ui, t, label, Emphasis::Primary);
            });
            discard = row_button(ui, t, cancel, Emphasis::Quiet);
        });
    });
    (save, discard)
}

pub fn pane(
    ui: &mut egui::Ui,
    store: &Store,
    draft: &mut Draft,
    t: &Tokens,
    s: &Strings,
    actions: &mut Vec<SettingsAction>,
) {
    let hooks = &store.webhooks;
    draft.reconcile(store);
    if !hooks.loaded && !hooks.loading() && hooks.error.is_none() {
        dispatch(actions, WebhookAction::Load);
    }
    let busy = hooks.loading();
    ui.horizontal(|ui| {
        ui.add_space(space::LG);
        ui.label(
            RichText::new(s.webhook_integrations)
                .font(px(11.5, "semibold"))
                .color(t.label_secondary),
        );
        if busy {
            ui.spinner();
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_enabled_ui(!busy && !draft.creating, |ui| {
                if row_button(ui, t, s.webhook_create, Emphasis::Quiet) {
                    draft.creating = true;
                    draft.editing = None;
                    draft.revoking = None;
                }
            });
        });
    });
    ui.add_space(10.0);

    if let Some(error) = &hooks.error {
        sheet_card(ui, t, |card| {
            card.section("", |ui| {
                ui.add(
                    egui::Label::new(RichText::new(error).font(text::body()).color(t.danger))
                        .wrap(),
                );
                ui.add_space(space::XS);
                ui.add(
                    egui::Label::new(
                        RichText::new(s.webhook_unconfirmed)
                            .font(text::footnote())
                            .color(t.label_secondary),
                    )
                    .wrap(),
                );
                ui.add_space(space::MD);
                ui.add_enabled_ui(!busy, |ui| {
                    if row_button(ui, t, s.inbox_refresh, Emphasis::Quiet) {
                        dispatch(actions, WebhookAction::Load);
                    }
                });
            })
        });
        ui.add_space(space::LG);
    }

    if let Some(created) = &hooks.created {
        section(ui, t, s.webhook_credential);
        sheet_card(ui, t, |card| {
            card.section("", |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new(s.webhook_token_once)
                            .font(text::body())
                            .color(t.label),
                    )
                    .wrap(),
                );
                ui.add_space(space::MD);
                ui.add(
                    egui::Label::new(
                        RichText::new(&created.token)
                            .font(text::mono())
                            .color(t.label_secondary),
                    )
                    .selectable(true)
                    .wrap(),
                );
                ui.add_space(space::LG);
                ui.horizontal_wrapped(|ui| {
                    if row_button(ui, t, s.copy_text, Emphasis::Primary) {
                        crate::platform::copy::text(ui.ctx(), created.token.clone());
                    }
                    if row_button(ui, t, s.webhook_token_dismiss, Emphasis::Quiet) {
                        actions.push(SettingsAction::Chat(ChatAction::DismissWebhookCredential));
                    }
                });
            })
        });
    }

    if draft.creating {
        section(ui, t, s.webhook_create);
        let channels: Vec<_> = store
            .channels
            .iter()
            .filter(|channel| matches!(channel.kind, ChannelKind::Text | ChannelKind::Voice))
            .collect();
        if !channels.iter().any(|channel| channel.id == draft.channel) {
            draft.channel = channels
                .first()
                .map(|channel| channel.id.clone())
                .unwrap_or_default();
        }
        sheet_card(ui, t, |card| {
            card.section(s.webhook_name, |ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    super::profile::inline_field(
                        ui,
                        t,
                        "webhook-name",
                        &mut draft.name,
                        s.webhook_name,
                        px(13.0, "regular"),
                        t.label,
                        80,
                        (1, 1),
                    );
                });
            });
            card.section(s.webhook_channel, |ui| {
                if channels.is_empty() {
                    ui.label(
                        RichText::new(s.no_channels_yet)
                            .font(text::body())
                            .color(t.label_secondary),
                    );
                } else {
                    let labels: Vec<String> = channels
                        .iter()
                        .map(|channel| {
                            format!(
                                "{} {}",
                                if channel.kind == ChannelKind::Voice {
                                    "♪"
                                } else {
                                    "#"
                                },
                                channel.name
                            )
                        })
                        .collect();
                    let options: Vec<_> = labels
                        .iter()
                        .enumerate()
                        .map(|(index, label)| (index, label.as_str()))
                        .collect();
                    let mut index = channels
                        .iter()
                        .position(|channel| channel.id == draft.channel)
                        .unwrap_or_default();
                    ui.add_enabled_ui(!busy, |ui| {
                        settings::dropdown_choice(ui, t, &mut index, &options);
                    });
                    draft.channel = channels[index].id.clone();
                }
            });
        });
        let ready = !busy
            && !draft.channel.is_empty()
            && !draft.name.trim().is_empty()
            && draft.name.chars().count() <= 80;
        let (save, cancel) = ui
            .add_enabled_ui(!busy, |ui| footer(ui, t, ready, s.webhook_create, s.cancel))
            .inner;
        if save {
            draft.submitted = Some(Submission::Create(
                hooks.items.iter().map(|hook| hook.id.clone()).collect(),
            ));
            dispatch(
                actions,
                WebhookAction::Create {
                    channel: draft.channel.clone(),
                    name: draft.name.trim().into(),
                },
            );
        }
        if cancel && !busy {
            draft.creating = false;
            draft.name.clear();
        }
        ui.add_space(space::LG);
    }

    if let Some(id) = draft.revoking.clone() {
        if let Some(hook) = hooks.items.iter().find(|hook| hook.id == id) {
            section(ui, t, &hook.name);
            sheet_card(ui, t, |card| {
                card.section("", |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(s.webhook_revoke_confirm)
                                .font(text::body())
                                .color(t.label),
                        )
                        .wrap(),
                    );
                    ui.add_space(space::LG);
                    ui.add_enabled_ui(!busy, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if row_button(ui, t, s.webhook_revoke, Emphasis::Danger) {
                                draft.submitted = Some(Submission::Revoke(id.clone()));
                                dispatch(actions, WebhookAction::Revoke { id: id.clone() });
                            }
                            if row_button(ui, t, s.cancel, Emphasis::Quiet) {
                                draft.revoking = None;
                            }
                        });
                    });
                })
            });
            ui.add_space(space::LG);
        } else {
            draft.revoking = None;
        }
    }

    if hooks.items.is_empty() {
        if hooks.loaded && !draft.creating {
            sheet_card(ui, t, |card| {
                card.section("", |ui| {
                    ui.label(
                        RichText::new(s.webhook_empty)
                            .font(px(13.0, "medium"))
                            .color(t.label),
                    );
                    ui.add_space(space::XS);
                    ui.add(
                        egui::Label::new(
                            RichText::new(s.webhook_empty_hint)
                                .font(px(12.0, "regular"))
                                .color(t.label_secondary),
                        )
                        .wrap(),
                    );
                })
            });
        }
    } else {
        sheet_card(ui, t, |card| {
            for hook in &hooks.items {
                card.section("", |ui| {
                    let (rect, response) = ui
                        .allocate_exact_size(Vec2::new(ui.available_width(), 42.0), Sense::click());
                    if response.hovered() {
                        ui.painter().rect_filled(
                            rect.expand2(Vec2::new(6.0, 3.0)),
                            egui::CornerRadius::same(6),
                            t.fill_soft,
                        );
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    let center = egui::pos2(rect.min.x + 16.0, rect.center().y);
                    ui.painter().circle_filled(center, 16.0, t.fill_soft);
                    ui.painter().text(
                        center,
                        egui::Align2::CENTER_CENTER,
                        icon::PLUG,
                        text::icon(16.0),
                        t.label_secondary,
                    );
                    let left = rect.min.x + 44.0;
                    let available = (rect.max.x - left - 70.0).max(20.0);
                    widgets::text_fit(
                        ui.painter(),
                        egui::pos2(left, rect.min.y + 10.0),
                        egui::Align2::LEFT_CENTER,
                        &hook.name,
                        px(13.0, "medium"),
                        t.label,
                        available,
                    );
                    let mut subtitle = store
                        .channel(&hook.channel_id)
                        .map(|channel| {
                            format!(
                                "{} {}",
                                if channel.kind == ChannelKind::Voice {
                                    "♪"
                                } else {
                                    "#"
                                },
                                channel.name
                            )
                        })
                        .unwrap_or_else(|| hook.channel_id.clone());
                    if let Some(name) = hook
                        .display_name
                        .as_deref()
                        .filter(|name| !name.is_empty() && *name != hook.name)
                    {
                        subtitle.push_str(" · ");
                        subtitle.push_str(name);
                    }
                    widgets::text_fit(
                        ui.painter(),
                        egui::pos2(left, rect.max.y - 10.0),
                        egui::Align2::LEFT_CENTER,
                        &subtitle,
                        px(12.0, "regular"),
                        t.label_secondary,
                        available,
                    );
                    let mut edit = response.clicked();
                    let mut revoke = false;
                    let mut controls = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(egui::Rect::from_min_max(
                                egui::pos2(rect.max.x - 60.0, rect.min.y),
                                rect.max,
                            ))
                            .layout(Layout::left_to_right(Align::Center)),
                    );
                    controls.add_enabled_ui(!busy, |ui| {
                        edit |= widgets::icon_button(ui, t, icon::PENCIL_SIMPLE, s.edit).clicked();
                        revoke =
                            widgets::icon_button(ui, t, icon::TRASH, s.webhook_revoke).clicked();
                    });
                    if edit && !busy {
                        draft.editing = Some((
                            hook.id.clone(),
                            hook.display_name.clone().unwrap_or_default(),
                        ));
                        draft.creating = false;
                        draft.revoking = None;
                    }
                    if revoke {
                        draft.revoking = Some(hook.id.clone());
                        draft.creating = false;
                        draft.editing = None;
                    }
                    if let Some((_, value)) =
                        draft.editing.as_mut().filter(|(id, _)| id == &hook.id)
                    {
                        ui.add_space(space::LG);
                        ui.label(
                            RichText::new(s.webhook_display_name.to_uppercase())
                                .font(px(10.0, "semibold"))
                                .color(t.label_tertiary),
                        );
                        ui.add_space(space::SM);
                        ui.add_enabled_ui(!busy, |ui| {
                            super::profile::inline_field(
                                ui,
                                t,
                                "webhook-display-name",
                                value,
                                &hook.name,
                                px(13.0, "regular"),
                                t.label,
                                100,
                                (1, 1),
                            );
                        });
                        ui.add_space(space::XS);
                        ui.add(
                            egui::Label::new(
                                RichText::new(s.webhook_display_hint)
                                    .font(px(11.5, "regular"))
                                    .color(t.label_secondary),
                            )
                            .wrap(),
                        );
                        let (save, cancel) = ui
                            .add_enabled_ui(!busy, |ui| {
                                footer(
                                    ui,
                                    t,
                                    value.chars().count() <= 100
                                        && value.as_str()
                                            != hook.display_name.as_deref().unwrap_or_default(),
                                    s.save,
                                    s.cancel,
                                )
                            })
                            .inner;
                        if save {
                            draft.submitted =
                                Some(Submission::Identity(hook.id.clone(), value.clone()));
                            dispatch(
                                actions,
                                WebhookAction::Identity {
                                    id: hook.id.clone(),
                                    display_name: value.clone(),
                                },
                            );
                        }
                        if cancel {
                            draft.editing = None;
                        }
                    }
                });
            }
        });
    }
}
