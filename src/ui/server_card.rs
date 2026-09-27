//! Cartão do servidor: o que é seu neste servidor, para qualquer membro.
//!
//! É a pastilha do servidor "descomprimida" — mesma largura, mesma borda de
//! cima, crescendo para baixo por cima da lista de canais. Traz o endereço,
//! os avisos de cada canal, marcar como lido e sair. A administração não
//! mora aqui: ela fica na engrenagem, que só existe para quem administra, no
//! mesmo canto em que estava na pastilha fechada.
//!
//! No celular é uma folha que sobe do pé da tela, como o cartão de perfil.

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, UiBuilder, Vec2};
use egui_phosphor::regular as icon;

use crate::i18n::Strings;
use crate::platform::menu::MenuCommand;
use crate::state::{ChannelKind, Presence, Store};

use super::settings::ChannelNotifyMode;
use super::shell::{ChatAction, UiState};
use super::theme::{radius, space, text, Tokens};
use super::widgets::photo;

const OPEN_SECONDS: f64 = 0.16;
const MARGIN: f32 = space::LG;
const ICON: f32 = 44.0;
const ROW_H: f32 = 32.0;
const ROW_H_TOUCH: f32 = 44.0;

#[derive(Clone, Debug)]
pub struct ServerCard {
    opened: f64,
    /// Altura medida no quadro anterior; sem ela o primeiro quadro sai
    /// invisível e a animação de abertura esconde a medição.
    body_height: Option<f32>,
    /// "Sair do servidor" foi tocado uma vez e pede confirmação.
    confirm_leave: bool,
}

/// Abre o cartão, ou fecha se ele já estava aberto — como os painéis.
pub fn toggle(state: &mut UiState, now: f64) {
    state.server_card = match state.server_card {
        Some(_) => None,
        None => Some(ServerCard {
            opened: now,
            body_height: None,
            confirm_leave: false,
        }),
    };
}

/// Retângulo do cartão: a pastilha é a base, com a largura dela; o cartão
/// cresce para baixo a partir da borda de cima, sem passar da janela.
pub fn place(pill: Rect, height: f32, screen: Rect) -> Rect {
    let bottom = (pill.min.y + height).min(screen.max.y - MARGIN).max(pill.max.y);
    Rect::from_min_max(pill.min, egui::pos2(pill.max.x, bottom))
}

/// Desenha o cartão aberto, se houver. `back` é o voltar do sistema deste
/// quadro; devolve `true` quando foi ele que o usou.
pub fn draw(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    back: bool,
) -> bool {
    let Some(mut card) = state.server_card.take() else {
        return false;
    };
    let Some(pill) = state.server_pill.filter(|_| !state.compact).or_else(|| {
        // No celular a pastilha pode nem estar na tela (a gaveta fechou):
        // a folha não precisa dela.
        state.compact.then_some(Rect::NOTHING)
    }) else {
        return false;
    };
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let compact = state.compact;
    let now = ctx.input(|input| input.time);
    let motion = ui.style().animation_time > 0.0;
    let progress = if motion {
        (((now - card.opened) / OPEN_SECONDS) as f32).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let eased = 1.0 - (1.0 - progress).powi(3);
    if progress < 1.0 || card.body_height.is_none() {
        ctx.request_repaint();
    }

    let wanted = card.body_height.unwrap_or(screen.height());
    let rect = if compact {
        super::profile::place_sheet(wanted, screen)
    } else {
        place(pill, wanted, screen)
    };

    let layer = egui::LayerId::new(egui::Order::Foreground, Id::new("cartao-do-servidor"));
    let mut close = back || ctx.input(|input| input.key_pressed(egui::Key::Escape));
    egui::Area::new(layer.id)
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .show(&ctx, |ui| {
            ui.set_min_size(screen.size());
            // Bloqueio da tela inteira, antes do cartão: fora dele, o clique
            // fecha e não chega à conversa.
            let blocker = ui.interact(screen, Id::new("cartao-do-servidor-bloqueio"), Sense::click_and_drag());
            if compact {
                ui.painter().rect_filled(
                    screen.expand2(Vec2::new(0.0, screen.height() * 0.25)),
                    CornerRadius::ZERO,
                    Color32::from_black_alpha((90.0 * eased) as u8),
                );
            }
            close |= blocker.clicked();

            // Desce a partir da pastilha; no celular, sobe do pé.
            let lift = if compact { rect.height() * 0.25 } else { -6.0 };
            ctx.set_transform_layer(
                layer,
                egui::emath::TSTransform::from_translation(Vec2::new(0.0, (1.0 - eased) * lift)),
            );
            ui.multiply_opacity(if card.body_height.is_some() { eased } else { 0.0 });

            let corners = if compact {
                CornerRadius { nw: radius::SHEET, ne: radius::SHEET, sw: 0, se: 0 }
            } else {
                CornerRadius::same(radius::SHEET)
            };
            ui.painter().add(ui.visuals().window_shadow.as_shape(rect, corners));
            ui.painter().rect(rect, corners, t.elevated_bg, Stroke::new(1.0, t.separator), egui::StrokeKind::Inside);
            ui.interact(rect, Id::new("cartao-do-servidor-superficie"), Sense::click());

            let body = ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                ui.set_clip_rect(rect);
                egui::ScrollArea::vertical()
                    .id_salt("cartao-do-servidor-corpo")
                    .max_height(rect.height())
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::ZERO;
                        contents(ui, store, state, t, s, &mut card, rect, compact, &mut close);
                    })
            });
            let content = body.inner.content_size.y;
            if card.body_height != Some(content) {
                card.body_height = Some(content);
                ctx.request_repaint();
            }
        });

    if !close {
        state.server_card = Some(card);
    }
    back
}

#[allow(clippy::too_many_arguments)]
fn contents(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    card: &mut ServerCard,
    rect: Rect,
    compact: bool,
    close: &mut bool,
) {
    let ctx = ui.ctx().clone();
    let width = rect.width();
    let row_h = if compact { ROW_H_TOUCH } else { ROW_H };
    let (name, description, blob) = match &store.server {
        Some(server) => (server.name.clone(), server.description.clone(), server.icon.clone()),
        None => ("Papo".to_owned(), None, None),
    };

    // --- Cabeçalho: ícone, nome, quem está aqui --------------------------
    if compact {
        let (grab_row, _) = ui.allocate_exact_size(Vec2::new(width, space::LG), Sense::hover());
        let grab = Rect::from_center_size(grab_row.center(), Vec2::new(36.0, 5.0));
        ui.painter().rect_filled(grab, CornerRadius::same(3), t.fill_medium);
    }
    let head_h = ICON + space::LG * 2.0;
    let (head, _) = ui.allocate_exact_size(Vec2::new(width, head_h), Sense::hover());
    let icon_rect = Rect::from_min_size(head.min + Vec2::new(space::LG, space::LG), Vec2::splat(ICON));
    let corners = CornerRadius::same(radius::CARD);
    let texture = blob
        .as_deref()
        .and_then(|blob| state.media.server_icon(blob))
        .and_then(|texture| texture.frame(&ctx))
        .map(|handle| handle.id());
    match texture {
        Some(texture) => photo(ui.painter(), icon_rect, texture, super::widgets::FULL_UV, corners, Color32::WHITE),
        None => {
            ui.painter().rect_filled(icon_rect, corners, t.accent.gamma_multiply(0.30));
            ui.painter().text(
                icon_rect.center(),
                Align2::CENTER_CENTER,
                initials(&name),
                egui::FontId::new(15.0, egui::FontFamily::Name("semibold".into())),
                t.accent,
            );
        }
    }

    // A engrenagem fica exatamente onde estava na pastilha fechada.
    let admin = store.can_open_server_admin();
    let gear = Rect::from_center_size(
        egui::pos2(rect.max.x - space::SM - 14.0, rect.min.y + 23.0 + if compact { space::LG } else { 0.0 }),
        Vec2::splat(28.0),
    );
    let text_left = icon_rect.max.x + space::MD;
    let text_right = if admin { gear.min.x - space::XS } else { head.max.x - space::LG };
    let clip = ui.painter().with_clip_rect(Rect::from_x_y_ranges(text_left..=text_right, head.y_range()));
    let text_width = text_right - text_left;
    super::widgets::text_fit(&clip, egui::pos2(text_left, icon_rect.min.y + 1.0), Align2::LEFT_TOP, &name, text::title3(), t.label, text_width);
    let mut line_y = icon_rect.min.y + 21.0;
    if let Some(description) = description.as_deref().filter(|text| !text.is_empty()) {
        super::widgets::text_fit(&clip, egui::pos2(text_left, line_y), Align2::LEFT_TOP, description, text::callout(), t.label_secondary, text_width);
        line_y += 16.0;
    }
    let online = store.members.iter().filter(|member| member.presence != Presence::Offline).count();
    clip.circle_filled(egui::pos2(text_left + 3.5, line_y + 6.0), 3.5, t.online);
    super::widgets::text_fit(
        &clip,
        egui::pos2(text_left + 11.0, line_y),
        Align2::LEFT_TOP,
        &format!("{online} {} · {} {}", s.count_online, store.members.len(), s.count_members),
        text::footnote(),
        t.label_secondary,
        text_width - 11.0,
    );
    if admin {
        let response = ui.interact(gear, Id::new("cartao-do-servidor-engrenagem"), Sense::click());
        if response.hovered() {
            ui.painter().rect_filled(gear, CornerRadius::same(radius::FIELD), t.fill_medium);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        ui.painter().text(
            gear.center(),
            Align2::CENTER_CENTER,
            icon::GEAR_SIX,
            text::icon(15.0),
            if response.hovered() { t.label } else { t.label_secondary },
        );
        if response.on_hover_text(s.server_settings).clicked() {
            state.pending.push(MenuCommand::ServerSettings);
            *close = true;
        }
    }

    // --- Endereço ----------------------------------------------------------
    section(ui, t, width, s.card_address, |ui| {
        let address = state.server_url.trim_start_matches("https://").trim_start_matches("http://").to_owned();
        let response = row(ui, t, row_h, None, &address, Some(icon::COPY), text::mono(), t.label_secondary, false);
        if response.clicked() {
            ui.ctx().copy_text(state.server_url.clone());
            state.error = Some((s.address_copied.to_owned(), ui.input(|input| input.time)));
        }
    });

    // --- Avisos por canal ----------------------------------------------------
    let channels: Vec<_> = store
        .channels
        .iter()
        .filter(|channel| channel.kind != ChannelKind::Category)
        .map(|channel| (channel.id.clone(), channel.name.clone(), channel.kind, ChannelNotifyMode::parse(&channel.notification_settings)))
        .collect();
    if !channels.is_empty() {
        section(ui, t, width, s.card_alerts, |ui| {
            for (id, name, kind, mode) in &channels {
                let glyph = if *kind == ChannelKind::Voice { icon::SPEAKER_HIGH } else { icon::HASH };
                let value = match mode {
                    ChannelNotifyMode::All => s.notify_all_short,
                    ChannelNotifyMode::Mentions => s.notify_mentions_short,
                    ChannelNotifyMode::Off => s.notify_off_short,
                };
                let (response, value_rect) = value_row(ui, t, row_h, glyph, name, value, *mode == ChannelNotifyMode::Off);
                // Menu suspenso: o segmentado de três opções não cabe na
                // largura da pastilha. Sai do lado do cartão (desktop) ou
                // por cima do valor tocado (celular).
                super::widgets::dropdown(&response, rect, value_rect, compact).show(|ui| {
                    for (option, label) in [
                        (ChannelNotifyMode::All, s.notify_all),
                        (ChannelNotifyMode::Mentions, s.notify_mentions),
                        (ChannelNotifyMode::Off, s.notify_off),
                    ] {
                        if super::widgets::menu_option(ui, t, label, option == *mode) {
                            if option != *mode {
                                state.actions.push(ChatAction::ChannelNotifications {
                                    channel_id: id.clone(),
                                    setting: option.wire(),
                                });
                            }
                            ui.close();
                        }
                    }
                });
            }
        });
    }

    // --- Ações ----------------------------------------------------------------
    section(ui, t, width, "", |ui| {
        if row(ui, t, row_h, Some(icon::CHECKS), s.menu_mark_all_read, None, text::body(), t.label, false).clicked() {
            state.actions.push(ChatAction::MarkServerRead);
            *close = true;
        }
        // O último servidor não sai: sem nenhum, não há para onde ir.
        if state.server_count > 1 {
            if card.confirm_leave {
                ui.add_space(space::XS);
                ui.horizontal(|ui| {
                    ui.add_space(space::SM);
                    ui.vertical(|ui| {
                        ui.set_width(width - space::MD * 2.0 - space::SM * 2.0);
                        ui.label(egui::RichText::new(format!("{} {name}?", s.leave_confirm)).font(text::headline()).color(t.label));
                        ui.add(egui::Label::new(egui::RichText::new(s.leave_hint).font(text::footnote()).color(t.label_secondary)).wrap());
                        ui.add_space(space::SM);
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = space::SM;
                            if pill_button(ui, t, s.cancel, false).clicked() {
                                card.confirm_leave = false;
                            }
                            if pill_button(ui, t, s.leave, true).clicked() {
                                state.actions.push(ChatAction::LeaveServer);
                                *close = true;
                            }
                        });
                        ui.add_space(space::SM);
                    });
                });
            } else if row(ui, t, row_h, Some(icon::SIGN_OUT), s.leave_server, None, text::body(), t.danger, true).clicked() {
                card.confirm_leave = true;
            }
        }
    });
    ui.add_space(space::SM);
}

/// Seção do cartão: fio em cima a partir do recuo do texto, legenda em
/// versalete (quando há) e as linhas.
fn section(ui: &mut egui::Ui, t: &Tokens, width: f32, caption: &str, contents: impl FnOnce(&mut egui::Ui)) {
    let top = ui.cursor().min.y;
    let left = ui.max_rect().min.x;
    ui.painter().line_segment(
        [egui::pos2(left + space::LG, top), egui::pos2(left + width, top)],
        Stroke::new(1.0, t.separator),
    );
    ui.add_space(space::MD);
    if !caption.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(space::LG);
            ui.label(egui::RichText::new(caption.to_uppercase()).font(text::caption()).color(t.label_tertiary));
        });
        ui.add_space(space::XS);
    }
    ui.horizontal(|ui| {
        ui.add_space(space::SM);
        ui.vertical(|ui| {
            ui.set_width(width - space::SM * 2.0);
            contents(ui);
        });
    });
    ui.add_space(space::SM);
}

/// Linha de ação: ícone à esquerda (opcional), rótulo, ícone à direita.
#[allow(clippy::too_many_arguments)]
fn row(
    ui: &mut egui::Ui,
    t: &Tokens,
    height: f32,
    lead: Option<&str>,
    label: &str,
    trail: Option<&str>,
    font: egui::FontId,
    color: Color32,
    danger: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(radius::FIELD), t.fill_soft);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let mut x = rect.min.x + space::SM;
    if let Some(lead) = lead {
        ui.painter().text(
            egui::pos2(x + 8.0, rect.center().y),
            Align2::CENTER_CENTER,
            lead,
            text::icon(15.0),
            if danger { t.danger } else { t.label_secondary },
        );
        x += 16.0 + space::MD;
    }
    let right = rect.max.x - space::SM - if trail.is_some() { 16.0 + space::SM } else { 0.0 };
    super::widgets::text_fit(ui.painter(), egui::pos2(x, rect.center().y), Align2::LEFT_CENTER, label, font, color, right - x);
    if let Some(trail) = trail {
        ui.painter().text(
            egui::pos2(rect.max.x - space::SM - 7.0, rect.center().y),
            Align2::CENTER_CENTER,
            trail,
            text::icon(14.0),
            t.label_secondary,
        );
    }
    response
}

/// Linha de canal: ícone, nome e o valor atual à direita, com a seta.
fn value_row(ui: &mut egui::Ui, t: &Tokens, height: f32, glyph: &str, name: &str, value: &str, muted: bool) -> (egui::Response, Rect) {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(radius::FIELD), t.fill_soft);
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    painter.text(egui::pos2(rect.min.x + space::SM + 7.0, rect.center().y), Align2::CENTER_CENTER, glyph, text::icon(14.0), t.label_secondary);
    let caret_x = rect.max.x - space::SM - 5.0;
    painter.text(egui::pos2(caret_x, rect.center().y), Align2::CENTER_CENTER, icon::CARET_RIGHT, text::icon(10.0), t.label_tertiary);
    let value_galley = painter.layout_no_wrap(value.to_owned(), text::callout(), t.label_secondary);
    let value_x = caret_x - 8.0 - value_galley.size().x;
    let value_rect = Rect::from_min_size(
        egui::pos2(value_x, rect.min.y),
        Vec2::new(caret_x + 5.0 - value_x, rect.height()),
    );
    painter.galley(
        egui::pos2(value_x, rect.center().y - value_galley.size().y / 2.0),
        value_galley,
        if muted { t.label_tertiary } else { t.label_secondary },
    );
    let name_left = rect.min.x + space::SM + 22.0;
    super::widgets::text_fit(painter, egui::pos2(name_left, rect.center().y), Align2::LEFT_CENTER, name, text::body(), t.label, value_x - space::SM - name_left);
    (response, value_rect)
}

fn pill_button(ui: &mut egui::Ui, t: &Tokens, label: &str, danger: bool) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_owned(), text::callout(), t.label);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(galley.size().x + space::LG * 2.0, 26.0), Sense::click());
    let fill = match (danger, response.hovered()) {
        (true, false) => t.danger.gamma_multiply(0.16),
        (true, true) => t.danger.gamma_multiply(0.26),
        (false, false) => t.fill_soft,
        (false, true) => t.fill_medium,
    };
    ui.painter().rect_filled(rect, CornerRadius::same(radius::CONTROL), fill);
    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, if danger { t.danger } else { t.label });
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Rect {
        Rect::from_min_size(egui::pos2(0.0, 0.0), Vec2::new(1200.0, 800.0))
    }

    #[test]
    fn card_keeps_the_pill_width_and_top_edge() {
        let pill = Rect::from_min_size(egui::pos2(72.0, 8.0), Vec2::new(216.0, 46.0));
        let rect = place(pill, 380.0, screen());
        assert_eq!(rect.min, pill.min);
        assert_eq!(rect.max.x, pill.max.x);
        assert_eq!(rect.height(), 380.0);
    }

    #[test]
    fn tall_card_stops_at_the_window_bottom() {
        let pill = Rect::from_min_size(egui::pos2(72.0, 8.0), Vec2::new(216.0, 46.0));
        let rect = place(pill, 5000.0, screen());
        assert_eq!(rect.min.y, pill.min.y);
        assert!(rect.max.y <= screen().max.y - MARGIN);
    }

    #[test]
    fn card_is_never_shorter_than_the_pill() {
        let pill = Rect::from_min_size(egui::pos2(72.0, 8.0), Vec2::new(216.0, 46.0));
        assert_eq!(place(pill, 10.0, screen()).max.y, pill.max.y);
    }
}
