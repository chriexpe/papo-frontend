//! Janela principal: três colunas (canais · conversa · membros).
//!
//! As barras funcionais (laterais, topo e caixa de mensagem) formam a camada
//! translúcida; a lista de mensagens é a camada de conteúdo, sempre opaca.

use chrono::{Datelike, Local};
use egui::{
    Align, Color32, CornerRadius, Frame, Id, Layout, Margin, Rect, RichText, Sense, Stroke,
    TextEdit, UiBuilder, Vec2,
};
use egui_phosphor::regular as icon;

use crate::i18n::Strings;
use crate::platform::menu::MenuCommand;
use crate::state::{ChannelKind, Message, Presence, Store};

use super::glass::SharedGlass;
use super::theme::{radius, space, text, Tokens, HIT_TARGET};
use super::widgets::{avatar, icon_button, scroll_edge_fade, section_caption, sidebar_frame};

pub const SIDEBAR_WIDTH: f32 = 232.0;
pub const MEMBERS_WIDTH: f32 = 196.0;
const SIDEBAR_HEADER_HEIGHT: f32 = 56.0;
const ACCOUNT_PILL_HEIGHT: f32 = 40.0;
/// Altura das pastilhas flutuantes e respiro entre elas e a borda.
const PILL_HEIGHT: f32 = 36.0;
const PILL_MARGIN: f32 = 12.0;
/// Raio das pastilhas flutuantes — o mesmo canto do realce interno.
const PILL_RADIUS: f32 = 12.0;
const ACTIONS_PILL_WIDTH: f32 = HIT_TARGET * 3.0 + space::XXS * 2.0 + space::XS * 2.0;
const GROUP_GAP_MINUTES: i64 = 5;

/// Estado que pertence à interface, não ao servidor.
pub struct UiState {
    pub composer: String,
    pub show_members: bool,
    pub translucent: bool,
    pub pending: Vec<MenuCommand>,
    /// Renderizador do vidro fosco; ausente quando o backend não é o glow.
    pub glass: Option<SharedGlass>,
    /// Mensagem pronta para sair, preenchida quando o usuário envia.
    pub outgoing: Option<String>,
    /// O texto mudou neste quadro (dispara o evento de digitação).
    pub typed: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            composer: String::new(),
            show_members: true,
            translucent: true,
            pending: Vec::new(),
            glass: None,
            outgoing: None,
            typed: false,
        }
    }
}

pub fn draw(ui: &mut egui::Ui, store: &mut Store, state: &mut UiState, t: &Tokens, s: &Strings) {
    channels_sidebar(ui, store, state, t, s);
    if state.show_members {
        members_sidebar(ui, store, t, s);
    }
    conversation(ui, store, state, t, s);
}

/// Desenha o fundo embaçado de uma barra: o que já foi pintado por baixo
/// dela entra borrado, e a tinta translúcida vem por cima.
fn glass_backdrop(ui: &egui::Ui, state: &UiState, rect: Rect, corner: f32) {
    if !state.translucent {
        return;
    }
    let Some(glass) = state.glass.clone() else {
        return;
    };
    let callback = eframe::egui_glow::CallbackFn::new(move |info, painter| {
        let viewport = info.viewport_in_pixels();
        if let Ok(mut glass) = glass.lock() {
            glass.render(
                painter.gl(),
                (
                    viewport.left_px,
                    viewport.top_px,
                    viewport.width_px,
                    viewport.height_px,
                ),
                info.screen_size_px[1] as i32,
                corner * info.pixels_per_point,
            );
        }
    });
    ui.painter().add(egui::PaintCallback {
        rect,
        callback: std::sync::Arc::new(callback),
    });
}

// ---------------------------------------------------------------------------
// Coluna esquerda — canais
// ---------------------------------------------------------------------------

fn channels_sidebar(
    root: &mut egui::Ui,
    store: &mut Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
) {
    egui::Panel::left("channels")
        .exact_size(SIDEBAR_WIDTH)
        .resizable(false)
        .frame(sidebar_frame(t))
        .show(root, |ui| {
            let full = ui.max_rect();

            // Cabeçalho do servidor.
            ui.scope_builder(
                UiBuilder::new().max_rect(Rect::from_min_size(
                    full.min,
                    Vec2::new(full.width(), SIDEBAR_HEADER_HEIGHT),
                )),
                |ui| {
                    ui.add_space(space::LG);
                    ui.horizontal(|ui| {
                        ui.add_space(space::XL);
                        ui.vertical(|ui| {
                            let (name, description) = match &store.server {
                                Some(server) => {
                                    (server.name.clone(), server.description.clone())
                                }
                                None => ("Papo".to_owned(), None),
                            };
                            ui.label(RichText::new(name).font(text::title3()).color(t.label));
                            if let Some(description) = description {
                                ui.label(
                                    RichText::new(description)
                                        .font(text::footnote())
                                        .color(t.label_tertiary),
                                );
                            }
                        });
                    });
                },
            );
            let header_bottom = full.min.y + SIDEBAR_HEADER_HEIGHT;
            ui.painter().line_segment(
                [
                    egui::pos2(full.min.x, header_bottom),
                    egui::pos2(full.max.x, header_bottom),
                ],
                Stroke::new(1.0, t.separator),
            );

            // A lista ocupa toda a altura restante: os canais passam por
            // baixo da pastilha flutuante da conta.
            let list = Rect::from_min_max(egui::pos2(full.min.x, header_bottom), full.max);

            ui.scope_builder(UiBuilder::new().max_rect(list), |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.add_space(space::XS);
                        let indent = space::LG;
                        ui.horizontal(|ui| {
                            ui.add_space(indent);
                            ui.vertical(|ui| {
                                section_caption(ui, t, s.text_channels);
                                for channel in store
                                    .channels
                                    .clone()
                                    .iter()
                                    .filter(|c| c.kind == ChannelKind::Text)
                                {
                                    if channel_row(
                                        ui,
                                        t,
                                        icon::HASH,
                                        &channel.name,
                                        store.selected_channel == channel.id,
                                        channel.unread,
                                        channel.mentions,
                                        SIDEBAR_WIDTH - indent * 2.0,
                                    ) {
                                        store.selected_channel = channel.id.clone();
                                    }
                                }

                                section_caption(ui, t, s.voice_channels);
                                for channel in store
                                    .channels
                                    .clone()
                                    .iter()
                                    .filter(|c| c.kind == ChannelKind::Voice)
                                {
                                    channel_row(
                                        ui,
                                        t,
                                        icon::SPEAKER_HIGH,
                                        &channel.name,
                                        false,
                                        false,
                                        0,
                                        SIDEBAR_WIDTH - indent * 2.0,
                                    );
                                }
                                // Espaço para a pastilha da conta não cobrir o
                                // último canal quando a lista chega ao fim.
                                ui.add_space(ACCOUNT_PILL_HEIGHT + space::XXL);
                            });
                        });
                    });
            });

            account_pill(ui, store, state, t, s, full);
        });
}

/// Pastilha flutuante da conta, no rodapé da barra lateral.
fn account_pill(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    sidebar: Rect,
) {
    let me = store.member(&store.me).cloned().unwrap_or(crate::state::Member {
        id: store.me.clone(),
        name: store.my_name.clone(),
        presence: crate::state::Presence::Online,
        role_color: None,
    });
    if me.name.is_empty() {
        return;
    }

    let label = ui
        .painter()
        .layout_no_wrap(me.name.clone(), text::headline(), t.label);
    let avatar_size = 26.0;
    let width = space::SM + avatar_size + space::MD + label.size().x + space::LG;
    let rect = Rect::from_min_size(
        egui::pos2(
            sidebar.min.x + space::LG,
            sidebar.max.y - ACCOUNT_PILL_HEIGHT - space::LG,
        ),
        Vec2::new(width.min(sidebar.width() - space::LG * 2.0), ACCOUNT_PILL_HEIGHT),
    );

    pill_surface(ui, state, t, rect);
    let response = ui.interact(rect, Id::new("account-pill"), Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(
            rect,
            CornerRadius::same(PILL_RADIUS as u8),
            t.fill_soft,
        );
    }

    let painter = ui.painter();
    let avatar_center = egui::pos2(rect.min.x + space::SM + avatar_size / 2.0, rect.center().y);
    let tint = me.role_color.unwrap_or(t.accent);
    painter.circle_filled(avatar_center, avatar_size / 2.0, tint.gamma_multiply(0.30));
    painter.text(
        avatar_center,
        egui::Align2::CENTER_CENTER,
        me.initials(),
        egui::FontId::new(10.0, egui::FontFamily::Name("semibold".into())),
        tint,
    );
    let dot = avatar_center + Vec2::splat(avatar_size / 2.0 * 0.72);
    painter.circle_filled(dot, 5.0, t.glass_opaque);
    painter.circle_filled(dot, 3.5, presence_color(t, me.presence));

    painter.galley(
        egui::pos2(
            avatar_center.x + avatar_size / 2.0 + space::MD,
            rect.center().y - label.size().y / 2.0,
        ),
        label,
        t.label,
    );

    let _ = s;
}

#[allow(clippy::too_many_arguments)]
fn channel_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    glyph: &str,
    name: &str,
    selected: bool,
    unread: bool,
    mentions: u32,
    width: f32,
) -> bool {
    let height = 30.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());

    if selected {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_medium);
    } else if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_soft);
    }

    let label_color = if selected || unread {
        t.label
    } else {
        t.label_secondary
    };
    let font = if unread && !selected {
        text::headline()
    } else {
        text::body()
    };

    let painter = ui.painter();
    painter.text(
        egui::pos2(rect.min.x + space::MD, rect.center().y),
        egui::Align2::LEFT_CENTER,
        glyph,
        text::icon(14.0),
        if selected { t.accent } else { t.label_tertiary },
    );
    painter.text(
        egui::pos2(rect.min.x + space::MD + 20.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        font,
        label_color,
    );

    if mentions > 0 {
        let pill_width = 20.0_f32.max(12.0 + mentions.to_string().len() as f32 * 7.0);
        let pill = Rect::from_center_size(
            egui::pos2(rect.max.x - space::MD - pill_width / 2.0, rect.center().y),
            Vec2::new(pill_width, 16.0),
        );
        painter.rect_filled(pill, CornerRadius::same(8), t.accent);
        painter.text(
            pill.center(),
            egui::Align2::CENTER_CENTER,
            mentions.to_string(),
            text::caption(),
            t.accent_label,
        );
    } else if unread {
        painter.circle_filled(
            egui::pos2(rect.min.x - space::SM * 0.5, rect.center().y),
            3.0,
            t.label,
        );
    }

    response.clicked()
}

// ---------------------------------------------------------------------------
// Coluna direita — membros
// ---------------------------------------------------------------------------

fn members_sidebar(root: &mut egui::Ui, store: &Store, t: &Tokens, s: &Strings) {
    egui::Panel::right("members")
        .exact_size(MEMBERS_WIDTH)
        .resizable(false)
        .frame(sidebar_frame(t))
        .show(root, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add_space(space::LG);
                        ui.vertical(|ui| {
                            for (presence, label) in [
                                (Presence::Online, s.online),
                                (Presence::Away, s.away),
                                (Presence::Busy, s.busy),
                                (Presence::Offline, s.offline),
                            ] {
                                let people: Vec<_> = store.members_by_presence(presence).collect();
                                if people.is_empty() {
                                    continue;
                                }
                                section_caption(
                                    ui,
                                    t,
                                    &format!("{label} — {}", people.len()),
                                );
                                for member in people {
                                    member_row(
                                        ui,
                                        t,
                                        &member.initials(),
                                        &member.name,
                                        presence_color(t, member.presence),
                                        member.role_color,
                                        presence == Presence::Offline,
                                        MEMBERS_WIDTH - space::LG * 2.0,
                                    );
                                }
                            }
                            ui.add_space(space::LG);
                        });
                    });
                });
        });
}

#[allow(clippy::too_many_arguments)]
fn member_row(
    ui: &mut egui::Ui,
    t: &Tokens,
    initials: &str,
    name: &str,
    dot: Color32,
    role_color: Option<Color32>,
    dimmed: bool,
    width: f32,
) {
    let height = 32.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_soft);
    }

    let alpha = if dimmed { 0.45 } else { 1.0 };
    let avatar_rect = Rect::from_center_size(
        egui::pos2(rect.min.x + space::SM + 11.0, rect.center().y),
        Vec2::splat(22.0),
    );
    let tint = role_color.unwrap_or(t.accent).gamma_multiply(alpha);
    let painter = ui.painter();
    painter.circle_filled(avatar_rect.center(), 11.0, tint.gamma_multiply(0.30));
    painter.text(
        avatar_rect.center(),
        egui::Align2::CENTER_CENTER,
        initials,
        egui::FontId::new(9.0, egui::FontFamily::Name("semibold".into())),
        tint,
    );
    painter.circle_filled(avatar_rect.right_bottom() - Vec2::splat(1.0), 4.5, t.glass_opaque);
    painter.circle_filled(avatar_rect.right_bottom() - Vec2::splat(1.0), 3.0, dot.gamma_multiply(alpha));
    painter.text(
        egui::pos2(avatar_rect.max.x + space::MD, rect.center().y),
        egui::Align2::LEFT_CENTER,
        name,
        text::body(),
        if dimmed { t.label_tertiary } else { t.label_secondary },
    );
}

// ---------------------------------------------------------------------------
// Centro — conversa
// ---------------------------------------------------------------------------

fn conversation(
    root: &mut egui::Ui,
    store: &mut Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
) {
    let frame = Frame::new().fill(t.content_bg);
    egui::CentralPanel::default().frame(frame).show(root, |ui| {
        let full = ui.max_rect();
        let composer_height = composer_height(&state.composer);
        let top_inset = PILL_MARGIN * 2.0 + PILL_HEIGHT;
        let bottom_inset = PILL_MARGIN * 2.0 + composer_height;

        // Camada de conteúdo: ocupa a janela inteira e corre por baixo das
        // pastilhas.
        ui.scope_builder(UiBuilder::new().max_rect(full), |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    ui.add_space(top_inset);
                    message_list(ui, store, t, s, full.width());
                    ui.add_space(bottom_inset);
                });
        });

        // O conteúdo se dissolve onde encontra a camada flutuante, em vez de
        // ser cortado por ela.
        scroll_edge_fade(
            ui,
            Rect::from_min_size(full.min, Vec2::new(full.width(), top_inset)),
            t.content_bg,
            true,
        );
        scroll_edge_fade(
            ui,
            Rect::from_min_size(
                egui::pos2(full.min.x, full.max.y - bottom_inset),
                Vec2::new(full.width(), bottom_inset),
            ),
            t.content_bg,
            false,
        );

        // Camada funcional: tudo flutua.
        connection_pill(ui, store, state, t, s, full);
        channel_pill(ui, store, state, t, full);
        actions_pill(ui, state, t, s, full);
        composer(ui, store, state, t, s, full, composer_height);
    });
}

/// Fundo de uma pastilha: vidro fosco, tinta translúcida e fio de contorno.
fn pill_surface(ui: &egui::Ui, state: &UiState, t: &Tokens, rect: Rect) {
    let radius = PILL_RADIUS;
    glass_backdrop(ui, state, rect, radius);
    ui.painter().rect(
        rect,
        CornerRadius::same(radius as u8),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    // Brilho de um pixel no topo, como nas barras da Apple.
    ui.painter().line_segment(
        [
            egui::pos2(rect.min.x + radius * 0.6, rect.min.y + 0.5),
            egui::pos2(rect.max.x - radius * 0.6, rect.min.y + 0.5),
        ],
        Stroke::new(1.0, t.glass_highlight),
    );
}

/// Aviso de conexão, centralizado no alto: só aparece quando o socket não
/// está de pé.
fn connection_pill(
    ui: &egui::Ui,
    store: &Store,
    state: &UiState,
    t: &Tokens,
    s: &Strings,
    area: Rect,
) {
    let (label, color) = match store.connection {
        crate::api::ws::Connection::Online => return,
        crate::api::ws::Connection::Connecting => (s.reconnecting, t.away),
        crate::api::ws::Connection::Offline => (s.offline_banner, t.danger),
    };

    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), text::footnote(), t.label_secondary);
    let height = 26.0;
    let width = galley.size().x + space::XXL + space::LG;
    let rect = Rect::from_center_size(
        egui::pos2(area.center().x, area.min.y + PILL_MARGIN + height / 2.0),
        Vec2::new(width, height),
    );

    glass_backdrop(ui, state, rect, height / 2.0);
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same((height / 2.0) as u8),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(
        egui::pos2(rect.min.x + space::LG, rect.center().y),
        3.5,
        color,
    );
    painter.galley(
        egui::pos2(
            rect.min.x + space::LG + space::LG,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        t.label_secondary,
    );
}

/// Pastilha pequena de "fulano está digitando", acima da caixa de texto.
fn typing_pill(
    ui: &egui::Ui,
    store: &Store,
    state: &UiState,
    t: &Tokens,
    s: &Strings,
    composer: Rect,
) {
    let names = store.typing_names();
    if names.is_empty() {
        return;
    }
    let verb = if names.len() == 1 {
        s.typing_one
    } else {
        s.typing_many
    };
    let label = ui.painter().layout_no_wrap(
        format!("{} {verb}", names.join(", ")),
        text::footnote(),
        t.label_secondary,
    );

    let height = 22.0;
    let rect = Rect::from_min_size(
        egui::pos2(composer.min.x, composer.min.y - height - space::XS),
        Vec2::new(label.size().x + space::LG * 2.0, height),
    );
    glass_backdrop(ui, state, rect, height / 2.0);
    ui.painter().rect(
        rect,
        CornerRadius::same((height / 2.0) as u8),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    ui.painter().galley(
        egui::pos2(rect.min.x + space::LG, rect.center().y - label.size().y / 2.0),
        label,
        t.label_secondary,
    );
}

/// Pastilha de identidade do canal, no alto à esquerda.
fn channel_pill(ui: &mut egui::Ui, store: &Store, state: &mut UiState, t: &Tokens, area: Rect) {
    let Some(channel) = store.channel(&store.selected_channel).cloned() else {
        return;
    };

    let painter = ui.painter();
    let glyph = painter.layout_no_wrap(icon::HASH.to_owned(), text::icon(15.0), t.label_tertiary);
    let name = painter.layout_no_wrap(channel.name.clone(), text::title3(), t.label);
    let topic = channel.topic.as_ref().map(|topic| {
        painter.layout_no_wrap(topic.clone(), text::callout(), t.label_tertiary)
    });

    let mut width = space::LG + glyph.size().x + space::SM + name.size().x + space::LG;
    if let Some(topic) = &topic {
        width += space::LG + 1.0 + space::LG + topic.size().x;
    }
    let width = width.min(area.width() - PILL_MARGIN * 2.0 - ACTIONS_PILL_WIDTH - space::MD);

    let rect = Rect::from_min_size(
        area.min + Vec2::splat(PILL_MARGIN),
        Vec2::new(width, PILL_HEIGHT),
    );
    pill_surface(ui, state, t, rect);

    let mid = rect.center().y;
    let mut x = rect.min.x + space::LG;
    let painter = ui.painter();
    painter.galley(egui::pos2(x, mid - glyph.size().y / 2.0), glyph.clone(), t.label_tertiary);
    x += glyph.size().x + space::SM;
    painter.galley(egui::pos2(x, mid - name.size().y / 2.0), name.clone(), t.label);
    x += name.size().x + space::LG;

    if let Some(topic) = topic {
        if x + space::LG + topic.size().x < rect.max.x {
            painter.line_segment(
                [egui::pos2(x, mid - 7.0), egui::pos2(x, mid + 7.0)],
                Stroke::new(1.0, t.separator),
            );
            x += space::LG;
            painter.galley(egui::pos2(x, mid - topic.size().y / 2.0), topic, t.label_tertiary);
        }
    }
}

/// Pastilha de ações do canal, no alto à direita.
fn actions_pill(ui: &mut egui::Ui, state: &mut UiState, t: &Tokens, s: &Strings, area: Rect) {
    let rect = Rect::from_min_size(
        egui::pos2(
            area.max.x - PILL_MARGIN - ACTIONS_PILL_WIDTH,
            area.min.y + PILL_MARGIN,
        ),
        Vec2::new(ACTIONS_PILL_WIDTH, PILL_HEIGHT),
    );
    pill_surface(ui, state, t, rect);

    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(space::XS, space::XS)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            ui.spacing_mut().item_spacing.x = space::XXS;
            if icon_button(ui, t, icon::MAGNIFYING_GLASS, s.search).clicked() {
                state.pending.push(MenuCommand::Search);
            }
            let _ = icon_button(ui, t, icon::PUSH_PIN, s.pinned);
            if icon_button(ui, t, icon::USERS, s.members).clicked() {
                state.pending.push(MenuCommand::ToggleMembers);
            }
        },
    );
}

fn message_list(ui: &mut egui::Ui, store: &Store, t: &Tokens, s: &Strings, width: f32) {
    let messages: Vec<&Message> = store.messages_in(&store.selected_channel).collect();
    if messages.is_empty() {
        empty_state(ui, t, s);
        return;
    }

    let gutter = space::XL;
    let avatar_size = 36.0;
    let text_indent = gutter + avatar_size + space::LG;
    let text_width = width - text_indent - space::XL;

    let mut last_author: Option<&str> = None;
    let mut last_at: Option<chrono::DateTime<Local>> = None;
    let mut last_day: Option<u32> = None;

    for message in messages {
        let day = message.at.day();
        if last_day != Some(day) {
            day_divider(ui, t, s, message.at, width);
            last_day = Some(day);
            last_author = None;
        }

        let grouped = last_author == Some(message.author_id.as_str())
            && last_at.is_some_and(|prev| {
                (message.at - prev).num_minutes() < GROUP_GAP_MINUTES
            });

        let author = store.member(&message.author_id);
        if grouped {
            ui.horizontal(|ui| {
                ui.add_space(text_indent);
                ui.vertical(|ui| {
                    ui.set_max_width(text_width);
                    message_body(ui, t, s, message);
                });
            });
        } else {
            ui.add_space(space::LG);
            ui.horizontal_top(|ui| {
                ui.add_space(gutter);
                let initials = author.map(|a| a.initials()).unwrap_or_else(|| "?".into());
                avatar(ui, t, &initials, avatar_size, author.and_then(|a| a.role_color));
                ui.add_space(space::LG);
                ui.vertical(|ui| {
                    ui.set_max_width(text_width);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(author.map(|a| a.name.as_str()).unwrap_or("?"))
                                .font(text::headline())
                                .color(author.and_then(|a| a.role_color).unwrap_or(t.label)),
                        );
                        ui.add_space(space::XS);
                        ui.label(
                            RichText::new(message.at.format("%H:%M").to_string())
                                .font(text::footnote())
                                .color(t.label_tertiary),
                        );
                    });
                    ui.add_space(space::XXS);
                    message_body(ui, t, s, message);
                });
            });
        }

        last_author = Some(&message.author_id);
        last_at = Some(message.at);
    }
}

fn message_body(ui: &mut egui::Ui, t: &Tokens, s: &Strings, message: &Message) {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &message.content,
        0.0,
        egui::TextFormat {
            font_id: text::message(),
            color: t.label,
            ..Default::default()
        },
    );
    if message.edited {
        job.append(
            &format!("  ({})", s.edited),
            0.0,
            egui::TextFormat {
                font_id: text::footnote(),
                color: t.label_tertiary,
                ..Default::default()
            },
        );
    }
    job.wrap.max_width = ui.available_width();
    ui.label(job);

    if !message.reactions.is_empty() {
        ui.add_space(space::XS);
        ui.horizontal(|ui| {
            for reaction in &message.reactions {
                reaction_chip(ui, t, &reaction.emoji, reaction.count, reaction.mine);
            }
        });
    }
}

fn reaction_chip(ui: &mut egui::Ui, t: &Tokens, emoji: &str, count: u32, mine: bool) {
    let label = format!("{emoji} {count}");
    let galley = ui.painter().layout_no_wrap(label.clone(), text::callout(), t.label);
    let size = Vec2::new(galley.size().x + space::LG, 22.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    let (fill, stroke, text_color) = if mine {
        (t.accent.gamma_multiply(0.20), t.accent, t.label)
    } else if response.hovered() {
        (t.fill_medium, t.separator, t.label)
    } else {
        (t.fill_soft, Color32::TRANSPARENT, t.label_secondary)
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(11),
        fill,
        Stroke::new(1.0, stroke),
        egui::StrokeKind::Inside,
    );
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        label,
        text::callout(),
        text_color,
    );
}

fn day_divider(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    at: chrono::DateTime<Local>,
    width: f32,
) {
    let today = Local::now().date_naive();
    let date = at.date_naive();
    let label = if date == today {
        s.today.to_owned()
    } else if today.signed_duration_since(date).num_days() == 1 {
        s.yesterday.to_owned()
    } else {
        at.format("%d/%m/%Y").to_string()
    };

    ui.add_space(space::XXL);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width - space::XL * 2.0, 16.0), Sense::hover());
    let galley = ui
        .painter()
        .layout_no_wrap(label, text::caption(), t.label_tertiary);
    let text_width = galley.size().x;
    let mid = rect.center();
    let painter = ui.painter();
    painter.line_segment(
        [
            egui::pos2(rect.min.x + space::XL, mid.y),
            egui::pos2(mid.x - text_width / 2.0 - space::LG, mid.y),
        ],
        Stroke::new(1.0, t.separator),
    );
    painter.line_segment(
        [
            egui::pos2(mid.x + text_width / 2.0 + space::LG, mid.y),
            egui::pos2(rect.max.x - space::XL, mid.y),
        ],
        Stroke::new(1.0, t.separator),
    );
    painter.galley(
        egui::pos2(mid.x - text_width / 2.0, mid.y - galley.size().y / 2.0),
        galley,
        t.label_tertiary,
    );
    ui.add_space(space::XS);
}

fn empty_state(ui: &mut egui::Ui, t: &Tokens, s: &Strings) {
    ui.add_space(space::XXXL * 2.0);
    ui.vertical_centered(|ui| {
        ui.label(
            RichText::new(icon::CHATS_CIRCLE)
                .font(text::icon(44.0))
                .color(t.label_tertiary),
        );
        ui.add_space(space::LG);
        ui.label(
            RichText::new(s.empty_channel_title)
                .font(text::title2())
                .color(t.label),
        );
        ui.add_space(space::XS);
        ui.label(
            RichText::new(s.empty_channel_body)
                .font(text::body())
                .color(t.label_secondary),
        );
    });
}

/// Tira o texto da caixa e o coloca na fila de envio.
fn submit(state: &mut UiState) {
    let content = state.composer.trim().to_owned();
    state.composer.clear();
    if !content.is_empty() {
        state.outgoing = Some(content);
    }
}

fn composer_height(text: &str) -> f32 {
    let lines = text.lines().count().clamp(1, 8) as f32;
    44.0 + (lines - 1.0) * 18.0
}

fn composer(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    area: Rect,
    height: f32,
) {
    let rect = Rect::from_min_max(
        egui::pos2(area.min.x + PILL_MARGIN, area.max.y - PILL_MARGIN - height),
        egui::pos2(area.max.x - PILL_MARGIN, area.max.y - PILL_MARGIN),
    );

    typing_pill(ui, store, state, t, s, rect);
    pill_surface(ui, state, t, rect);

    let channel_name = store
        .channel(&store.selected_channel)
        .map(|c| c.name.clone())
        .unwrap_or_default();

    ui.scope_builder(
        UiBuilder::new()
            .max_rect(rect.shrink2(Vec2::new(space::MD, space::SM)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            let _ = icon_button(ui, t, icon::PAPERCLIP, s.attach);
            ui.add_space(space::XXS);

            let has_text = !state.composer.trim().is_empty();
            let send_width = HIT_TARGET + space::XS;
            let text_width = ui.available_width() - send_width;

            ui.scope_builder(
                UiBuilder::new().max_rect(Rect::from_min_size(
                    ui.cursor().min,
                    Vec2::new(text_width, ui.available_height()),
                )),
                |ui| {
                    let hint = format!("{} #{}…", s.composer_hint, channel_name);
                    let response = ui.add(
                        TextEdit::multiline(&mut state.composer)
                            .hint_text(RichText::new(hint).color(t.label_tertiary))
                            .frame(Frame::NONE)
                            .font(text::message())
                            .desired_rows(1)
                            .vertical_align(Align::Center)
                            .desired_width(f32::INFINITY)
                            .margin(Margin::symmetric(space::XS as i8, space::SM as i8)),
                    );
                    state.typed = response.changed();
                    // Enter envia; Shift+Enter quebra linha.
                    let enter = response.has_focus()
                        && ui.input(|input| {
                            input.key_pressed(egui::Key::Enter) && !input.modifiers.shift
                        });
                    if enter {
                        submit(state);
                    }
                },
            );

            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let (send_rect, response) =
                    ui.allocate_exact_size(Vec2::splat(HIT_TARGET), Sense::click());
                let fill = if has_text {
                    t.accent
                } else {
                    Color32::TRANSPARENT
                };
                ui.painter().circle_filled(send_rect.center(), 13.0, fill);
                ui.painter().text(
                    send_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    icon::PAPER_PLANE_TILT,
                    text::icon(14.0),
                    if has_text { t.accent_label } else { t.label_tertiary },
                );
                if response.clicked() && has_text {
                    submit(state);
                }
            });
        },
    );
}

// ---------------------------------------------------------------------------

pub fn presence_color(t: &Tokens, presence: Presence) -> Color32 {
    match presence {
        Presence::Online => t.online,
        Presence::Away => t.away,
        Presence::Busy => t.busy,
        Presence::Offline => t.label_tertiary,
    }
}

const _: () = {
    // `Id` e `Frame` entram na assinatura pública de helpers futuros.
    #[allow(dead_code)]
    fn _unused(_: Id, _: Frame) {}
};
