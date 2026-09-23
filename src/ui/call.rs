//! A call na tela.
//!
//! São três formas do mesmo estado, e a call passa de uma para a outra sem
//! cortar o áudio:
//!
//! - **no canal**, quando é só voz: a grade ocupa a conversa e a coluna da
//!   esquerda ganha a barra de conectado. É o lugar natural de uma sala de
//!   voz — ela pertence ao canal;
//! - **na folha**, assim que aparece vídeo: o vídeo pede a tela toda, mas
//!   não pode levar a conversa junto, então ele vem por cima, em vidro, e
//!   encolhe numa pastilha quando se quer escrever;
//! - **em janela própria**, quando se pede: para jogar noutro monitor.
//!
//! A grade e os controles são os mesmos nas três — o que muda é a moldura.

use egui::{Color32, CornerRadius, Rect, Sense, Stroke, Vec2};
use egui_phosphor::regular as icon;

use crate::i18n::Strings;
use crate::state::{Stage, Store};
use crate::voice::{Call, Kind};

use super::shell::{ChatAction, UiState};
use super::theme::{radius, space, text, Tokens};
use super::widgets::avatar;

/// Altura da barra de conectado, no pé da coluna de canais.
pub const BAR_HEIGHT: f32 = 52.0;

/// Proporção usada só enquanto ainda não existe um quadro para medir.
/// Assim que chega vídeo, a textura é a fonte de verdade: Android pode
/// publicar 3:4 enquanto uma webcam de desktop continua em 16:9.
const TILE_RATIO: f32 = 16.0 / 9.0;

/// O que a grade precisa saber de cada pessoa na sala.
struct Face {
    id: String,
    name: String,
    initials: String,
    tint: Option<Color32>,
    muted: bool,
    camera: bool,
    speaking: bool,
    me: bool,
}

fn rgb([r, g, b]: [u8; 3]) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn faces(store: &Store) -> Vec<Face> {
    store
        .call
        .members()
        .iter()
        .map(|member| {
            let person = store.member(&member.user_id);
            let name = person
                .map(|person| person.name.clone())
                .unwrap_or_else(|| member.user_id.clone());
            Face {
                id: member.user_id.clone(),
                initials: person
                    .map(crate::state::Member::initials)
                    .unwrap_or_else(|| name.chars().take(2).collect::<String>().to_uppercase()),
                name,
                tint: person.and_then(|person| person.role_color.map(rgb)),
                muted: member.muted,
                camera: member.camera_on,
                speaking: store.call.speaking(&member.user_id),
                me: member.user_id == store.me,
            }
        })
        .collect()
}

/// Participantes logo abaixo do canal de voz, na coluna da esquerda.
pub fn roster(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    channel_id: &str,
    width: f32,
) {
    let ctx = ui.ctx().clone();
    for member in store.call.room(channel_id) {
        let person = store.member(&member.user_id);
        let name = person
            .map(|person| person.name.clone())
            .unwrap_or_else(|| s.call_you.to_owned());
        let initials = person
            .map(crate::state::Member::initials)
            .unwrap_or_default();
        let texture = state
            .media
            .avatar(
                &member.user_id,
                store.avatars.get(&member.user_id).map(String::as_str),
            )
            .and_then(|texture| texture.frame(&ctx))
            .map(|handle| handle.id());

        let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::hover());
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect.shrink2(Vec2::new(space::XXL, 0.0)))
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        let speaking = store.call.speaking(&member.user_id);
        let ring = avatar(&mut child, t, &initials, 18.0, person.and_then(|p| p.role_color.map(rgb)), texture);
        if speaking {
            child.painter().circle_stroke(
                ring.rect.center(),
                ring.rect.width() / 2.0 + 1.5,
                Stroke::new(1.5, t.online),
            );
        }
        child.add_space(space::SM);
        child.label(
            egui::RichText::new(name)
                .font(text::caption())
                .color(if speaking { t.label } else { t.label_secondary }),
        );
        if member.muted {
            child.add_space(space::XS);
            child.label(
                egui::RichText::new(icon::MICROPHONE_SLASH)
                    .font(text::icon(11.0))
                    .color(t.label_tertiary),
            );
        }
        if member.camera_on {
            child.add_space(space::XXS);
            child.label(
                egui::RichText::new(icon::VIDEO_CAMERA)
                    .font(text::icon(11.0))
                    .color(t.label_tertiary),
            );
        }
    }
}

/// Barra de conectado, acima da pastilha da conta: onde se está, se a mídia
/// já subiu, e o caminho mais curto para mudo, câmera e sair.
pub fn bar(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    rect: Rect,
    live: bool,
) {
    let back = ui.interact(rect, egui::Id::new("barra-da-call"), Sense::click());
    if back.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(radius::CARD),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    if back.hovered() {
        painter.rect_filled(rect, CornerRadius::same(radius::CARD), t.fill_soft);
    }

    let channel = store
        .channel(&store.call.channel_id)
        .map(|channel| channel.name.clone())
        .unwrap_or_default();
    let (dot, label) = if live {
        (t.online, s.call_connected)
    } else {
        (t.away, s.call_connecting)
    };
    painter.circle_filled(
        egui::pos2(rect.min.x + space::LG, rect.min.y + space::LG + 1.0),
        3.5,
        dot,
    );
    painter.text(
        egui::pos2(rect.min.x + space::LG + 9.0, rect.min.y + space::LG + 1.0),
        egui::Align2::LEFT_CENTER,
        label,
        text::caption(),
        t.label_secondary,
    );
    painter.text(
        egui::pos2(rect.min.x + space::LG, rect.max.y - space::LG - 1.0),
        egui::Align2::LEFT_CENTER,
        format!("{} {}", icon::SPEAKER_HIGH, channel),
        text::caption(),
        t.label,
    );

    // Os três controles, encostados na direita.
    let size = 26.0;
    let mut x = rect.max.x - space::MD - size / 2.0;
    for (glyph, tip, on, danger, action) in [
        (
            icon::PHONE_X,
            s.call_leave,
            false,
            true,
            ChatAction::LeaveVoice,
        ),
        (
            if store.call.camera {
                icon::VIDEO_CAMERA
            } else {
                icon::VIDEO_CAMERA_SLASH
            },
            s.call_camera,
            store.call.camera,
            false,
            ChatAction::ToggleCamera,
        ),
        (
            if store.call.muted {
                icon::MICROPHONE_SLASH
            } else {
                icon::MICROPHONE
            },
            if store.call.muted {
                s.call_mic_off
            } else {
                s.call_mic
            },
            !store.call.muted,
            false,
            ChatAction::ToggleMute,
        ),
    ] {
        let spot = Rect::from_center_size(
            egui::pos2(x, rect.center().y),
            Vec2::splat(size),
        );
        if round_button(ui, t, spot, glyph, tip, on, danger) {
            state.actions.push(action);
        }
        x -= size + space::XS;
    }

    if back.clicked() {
        state.actions.push(ChatAction::OpenCall);
    }
}

/// A call ocupando a área da conversa (só voz, ou a folha encolhida).
pub fn dock(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    call: Option<&mut Call>,
    t: &Tokens,
    s: &Strings,
) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, t.content_bg);
    let controls = Rect::from_min_max(
        egui::pos2(rect.min.x, rect.max.y - 72.0),
        rect.max,
    );
    let stage = Rect::from_min_max(rect.min, egui::pos2(rect.max.x, controls.min.y))
        .shrink(space::XXL);
    grid(ui, store, state, call, t, s, stage);
    controls_row(ui, store, state, t, s, controls, false);
}

/// O canal de voz quando a grade não está aqui: ou você está de fora e é
/// convidado a entrar, ou a call está noutro lugar da tela — na folha, numa
/// janela — e o botão a traz de volta.
///
/// A diferença importa: oferecer "entrar" a quem já está dentro fazia o
/// clique sair da própria call e entrar de novo nela.
pub fn lobby(ui: &mut egui::Ui, store: &Store, state: &mut UiState, t: &Tokens, s: &Strings) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, t.content_bg);

    let channel_id = store.selected_channel.clone();
    let here = store.call.active() && store.call.channel_id == channel_id;
    let people = store.call.room(&channel_id).len();
    let middle = rect.center();

    ui.painter().text(
        egui::pos2(middle.x, middle.y - 46.0),
        egui::Align2::CENTER_CENTER,
        icon::SPEAKER_HIGH,
        text::icon(30.0),
        t.label_tertiary,
    );
    ui.painter().text(
        egui::pos2(middle.x, middle.y - 8.0),
        egui::Align2::CENTER_CENTER,
        if here {
            format!("{} · {people} {}", s.call_connected, s.call_in_channel)
        } else if people == 0 {
            s.call_empty_room.to_owned()
        } else {
            format!("{people} {}", s.call_in_channel)
        },
        text::body(),
        t.label_secondary,
    );

    let (label, action) = match (here, store.call.popped_out) {
        (true, true) => (s.call_popin, ChatAction::PopOutCall(false)),
        (true, false) => (s.call_expand, ChatAction::OpenCall),
        (false, _) => (s.call_join, ChatAction::JoinVoice(channel_id)),
    };

    let button = Rect::from_center_size(
        egui::pos2(middle.x, middle.y + 34.0),
        Vec2::new(184.0, 36.0),
    );
    let response = ui.interact(button, egui::Id::new("entrar-na-call"), Sense::click());
    let fill = if response.hovered() {
        t.accent
    } else {
        t.accent.gamma_multiply(0.85)
    };
    ui.painter()
        .rect_filled(button, CornerRadius::same(18), fill);
    ui.painter().text(
        button.center(),
        egui::Align2::CENTER_CENTER,
        label,
        text::body(),
        t.accent_label,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        state.actions.push(action);
    }
}

/// A folha de vidro por cima da conversa: é para onde a call vai quando
/// aparece vídeo.
pub fn sheet(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    call: Option<&mut Call>,
    t: &Tokens,
    s: &Strings,
    area: Rect,
) {
    let sheet = area.shrink2(Vec2::new(area.width() * 0.06, area.height() * 0.08));
    // Vidro grande é vidro quase opaco: o guia é explícito, e vídeo atrás de
    // textura vira ruído.
    ui.painter().rect(
        sheet,
        CornerRadius::same(radius::SHEET),
        t.elevated_bg.gamma_multiply(0.98),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    let title = store
        .channel(&store.call.channel_id)
        .map(|channel| channel.name.clone())
        .unwrap_or_default();
    let header = Rect::from_min_size(sheet.min, Vec2::new(sheet.width(), 44.0));
    ui.painter().text(
        egui::pos2(header.min.x + space::XL, header.center().y),
        egui::Align2::LEFT_CENTER,
        format!("{title} · {}", store.call.members().len()),
        text::body(),
        t.label,
    );

    let mut x = header.max.x - space::LG - 13.0;
    for (glyph, tip, action) in [
        (icon::ARROWS_IN, s.call_overlay, ChatAction::FloatCall(true)),
        (
            icon::ARROW_SQUARE_OUT,
            s.call_popout,
            ChatAction::PopOutCall(true),
        ),
    ] {
        let spot = Rect::from_center_size(egui::pos2(x, header.center().y), Vec2::splat(26.0));
        if round_button(ui, t, spot, glyph, tip, false, false) {
            state.actions.push(action);
        }
        x -= 26.0 + space::XS;
    }

    let controls = Rect::from_min_max(
        egui::pos2(sheet.min.x, sheet.max.y - 72.0),
        sheet.max,
    );
    let stage = Rect::from_min_max(
        egui::pos2(sheet.min.x, header.max.y),
        egui::pos2(sheet.max.x, controls.min.y),
    )
    .shrink(space::XL);
    grid(ui, store, state, call, t, s, stage);
    controls_row(ui, store, state, t, s, controls, true);
}

/// A pastilha compacta da call. No overlay móvel ela também carrega
/// "trazer de volta" e o seletor 1/2/4.
fn compact_pill(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    area: Rect,
    floating: bool,
) -> Rect {
    let button = 26.0;
    let button_gap = space::XXS;
    // mic + câmera + abrir/trazer + sair; no overlay entra também 1/2/4.
    let control_count = if floating { 5.0 } else { 4.0 };
    let controls_width = button * control_count + button_gap * (control_count - 1.0);
    let controls_only = space::MD * 2.0 + controls_width;

    let avatar_size = 22.0;
    let avatar_gap = 4.0;
    let speaker_chrome = space::SM * 2.0 + 1.0;
    let max_width = (area.width() * 0.56 - space::SM)
        .clamp(controls_only, 320.0);
    let speaker_budget = (max_width - controls_only - speaker_chrome).max(0.0);
    let max_speakers = (((speaker_budget + avatar_gap) / (avatar_size + avatar_gap)).floor()
        as usize)
        .min(store.call.speakers.len());
    let speakers_width = if max_speakers == 0 {
        0.0
    } else {
        avatar_size * max_speakers as f32 + avatar_gap * (max_speakers - 1) as f32
    };
    let width = controls_only
        + if max_speakers == 0 {
            0.0
        } else {
            speaker_chrome + speakers_width
        };

    let rect = Rect::from_min_size(
        egui::pos2(area.center().x - width / 2.0, area.min.y + space::LG),
        Vec2::new(width, 36.0),
    );
    let back = ui.interact(rect, egui::Id::new(("pastilha-da-call", floating)), Sense::click());
    if back.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    super::shell::glass_backdrop(ui, state, rect, 18.0);
    ui.painter().rect(
        rect,
        CornerRadius::same(18),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    // Um brilho curto no bordo de cima dá à cápsula a mesma leitura de
    // vidro das outras pastilhas, sem desenhar uma segunda moldura inteira.
    ui.painter().line_segment(
        [
            egui::pos2(rect.min.x + 18.0, rect.min.y + 0.75),
            egui::pos2(rect.max.x - 18.0, rect.min.y + 0.75),
        ],
        Stroke::new(1.0, t.glass_highlight),
    );
    if back.hovered() {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(18), t.fill_soft);
    }

    let divider_x = rect.max.x - space::MD - controls_width - space::SM;
    if max_speakers > 0 {
        ui.painter().line_segment(
            [
                egui::pos2(divider_x, rect.min.y + space::MD),
                egui::pos2(divider_x, rect.max.y - space::MD),
            ],
            Stroke::new(1.0, t.separator),
        );
    }

    let mut x = rect.min.x + space::SM + avatar_size / 2.0;
    let ctx = ui.ctx().clone();
    for user_id in store.call.speakers.iter().take(max_speakers) {
        let person = store.member(user_id);
        let initials = person
            .map(crate::state::Member::initials)
            .unwrap_or_default();
        let texture = state
            .media
            .avatar(user_id, store.avatars.get(user_id).map(String::as_str))
            .and_then(|texture| texture.frame(&ctx))
            .map(|handle| handle.id());
        let avatar_rect =
            Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::splat(avatar_size));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(avatar_rect)
                .layout(egui::Layout::centered_and_justified(egui::Direction::TopDown)),
        );
        let ring = avatar(
            &mut child,
            t,
            &initials,
            avatar_size,
            person.and_then(|p| p.role_color.map(rgb)),
            texture,
        );
        child.painter().circle_stroke(
            ring.rect.center(),
            ring.rect.width() / 2.0 + 1.5,
            Stroke::new(1.5, t.online),
        );
        x += avatar_size + avatar_gap;
    }

    let mut x = rect.max.x - space::MD - button / 2.0;
    let hangup = Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::splat(button));
    if round_button(ui, t, hangup, icon::PHONE_X, s.call_leave, false, true) {
        state.actions.push(ChatAction::LeaveVoice);
    }
    x -= button + button_gap;

    let return_or_expand = Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::splat(button));
    if round_button(
        ui,
        t,
        return_or_expand,
        icon::ARROWS_OUT,
        if floating { s.call_overlay_close } else { s.call_expand },
        false,
        false,
    ) {
        state.actions.push(if floating {
            ChatAction::FloatCall(false)
        } else {
            ChatAction::OpenCall
        });
    }
    x -= button + button_gap;

    if floating {
        let spot = Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::splat(button));
        let response = ui.interact(spot, egui::Id::new("quantidade-de-videos"), Sense::click());
        if response.hovered() {
            ui.painter().circle_filled(spot.center(), button / 2.0, t.fill_soft);
        }
        ui.painter().text(
            spot.center(),
            egui::Align2::CENTER_CENTER,
            state.call_video_tiles.to_string(),
            text::caption(),
            t.label,
        );
        if response.clicked() {
            state.call_video_tiles = match state.call_video_tiles {
                1 => 2,
                2 => 4,
                _ => 1,
            };
        }
        x -= button + button_gap;
    }

    let camera = Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::splat(button));
    if round_button(
        ui,
        t,
        camera,
        if store.call.camera {
            icon::VIDEO_CAMERA
        } else {
            icon::VIDEO_CAMERA_SLASH
        },
        if store.call.camera {
            s.call_camera_off
        } else {
            s.call_camera
        },
        store.call.camera,
        false,
    ) {
        state.actions.push(ChatAction::ToggleCamera);
    }
    x -= button + button_gap;

    let mic = Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::splat(button));
    if round_button(
        ui,
        t,
        mic,
        if store.call.muted {
            icon::MICROPHONE_SLASH
        } else {
            icon::MICROPHONE
        },
        s.call_mic,
        !store.call.muted,
        false,
    ) {
        state.actions.push(ChatAction::ToggleMute);
    }

    if back.clicked() && !floating {
        state.actions.push(ChatAction::OpenCall);
    }
    rect
}

/// A pastilha da folha encolhida: a call continua, fora do caminho.
pub fn pill(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    area: Rect,
) {
    compact_pill(ui, store, state, t, s, area, false);
}

/// No layout compacto, vídeo fica colado logo abaixo da mesma pastilha da
/// call. Não há cabeçalho intermediário nem espaço morto.
pub fn floating(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    call: Option<&mut Call>,
    t: &Tokens,
    s: &Strings,
    area: Rect,
) {
    let pill = compact_pill(ui, store, state, t, s, area, true);
    let limit: usize = match state.call_video_tiles {
        1 => 1,
        4 => 4,
        _ => 2,
    };
    // O painel desce o suficiente para não encostar nas duas pastilhas
    // vizinhas. O espaço entre ele e a cápsula fica limpo, sem ornamentos.
    let width = (area.width() - space::XXL * 2.0).clamp(220.0, 420.0);
    let columns = if limit == 1 { 1 } else { 2 };
    let rows = limit.div_ceil(columns);
    let gap = space::SM;
    let cell_width = (width - gap * (columns as f32 - 1.0)) / columns as f32;
    // O overlay compacto precisa reservar altura suficiente para câmera
    // em pé. O tile em si usa a proporção real da textura; aqui usamos a
    // proporção mais alta que o Android publica como reserva, para não
    // obrigar um quadro 3:4 a caber numa faixa 16:9 antes mesmo de ser
    // desenhado.
    #[cfg(target_os = "android")]
    let panel_ratio = 3.0 / 4.0;
    #[cfg(not(target_os = "android"))]
    let panel_ratio = TILE_RATIO;
    let height = rows as f32 * (cell_width / panel_ratio)
        + gap * (rows as f32 - 1.0)
        + space::SM * 2.0;
    let bridge_gap = 18.0;
    let rect = Rect::from_min_size(
        egui::pos2(area.center().x - width / 2.0, pill.max.y + bridge_gap),
        Vec2::new(width, height),
    );

    super::shell::glass_backdrop(ui, state, rect, radius::SHEET as f32);
    ui.painter().rect(
        rect,
        CornerRadius::same(radius::SHEET),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    ui.painter().line_segment(
        [
            egui::pos2(rect.min.x + radius::SHEET as f32, rect.min.y + 0.75),
            egui::pos2(rect.max.x - radius::SHEET as f32, rect.min.y + 0.75),
        ],
        Stroke::new(1.0, t.glass_highlight),
    );
    compact_grid(
        ui,
        store,
        state,
        call,
        t,
        s,
        rect.shrink(space::SM),
        limit,
    );
}

/// O conteúdo da janela só da call.
pub fn window(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    call: Option<&mut Call>,
    t: &Tokens,
    s: &Strings,
) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, t.content_bg);
    let controls = Rect::from_min_max(egui::pos2(rect.min.x, rect.max.y - 72.0), rect.max);
    let stage = Rect::from_min_max(rect.min, egui::pos2(rect.max.x, controls.min.y))
        .shrink(space::LG);
    grid(ui, store, state, call, t, s, stage);
    controls_row(ui, store, state, t, s, controls, true);
}

/// Conteúdo mínimo usado pelo Picture-in-Picture do Android.
/// A janela já é minúscula e os controles ficam na notificação da call, então
/// só a grade de vídeo ocupa a superfície.
#[cfg(target_os = "android")]
pub fn pip(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    call: Option<&mut Call>,
    t: &Tokens,
    s: &Strings,
) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, t.content_bg);

    // PiP não é uma miniatura da grade inteira. Ele acompanha quem está
    // falando; quando ninguém fala, prefere alguém que realmente tenha vídeo.
    // Assim a janela pequena continua legível e útil numa call com várias
    // pessoas em vez de virar quatro selos microscópicos.
    let mut people = faces(store);
    people.sort_by_key(|face| {
        let speaker_rank = store
            .call
            .speakers
            .iter()
            .position(|speaker| speaker == &face.id)
            .unwrap_or(usize::MAX);
        let video_rank = if face.camera { 0 } else { 1 };
        (speaker_rank, video_rank)
    });

    if !people.iter().any(|face| face.speaking) {
        people.sort_by_key(|face| if face.camera { 0 } else { 1 });
    }
    people.truncate(1);

    draw_faces(ui, store, state, call, t, s, rect, &people, None);
}

/// A grade: uma pessoa por retrato, vídeo quando há, foto quando não há.
fn grid(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    call: Option<&mut Call>,
    t: &Tokens,
    s: &Strings,
    area: Rect,
) {
    let people = faces(store);
    draw_faces(ui, store, state, call, t, s, area, &people, None);
}

fn compact_grid(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    call: Option<&mut Call>,
    t: &Tokens,
    s: &Strings,
    area: Rect,
    limit: usize,
) {
    let mut people = faces(store);
    people.sort_by_key(|face| {
        store
            .call
            .speakers
            .iter()
            .position(|speaker| speaker == &face.id)
            .unwrap_or(usize::MAX)
    });
    people.truncate(limit);
    let columns = if people.len() >= 2 { Some(2) } else { Some(1) };
    draw_faces(ui, store, state, call, t, s, area, &people, columns);
}

#[allow(clippy::too_many_arguments)]
fn draw_faces(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    mut call: Option<&mut Call>,
    t: &Tokens,
    s: &Strings,
    area: Rect,
    people: &[Face],
    forced_columns: Option<usize>,
) {
    if people.is_empty() {
        ui.painter().text(
            area.center(),
            egui::Align2::CENTER_CENTER,
            s.call_empty_room,
            text::body(),
            t.label_tertiary,
        );
        return;
    }
    if people.len() == 1 && people[0].me {
        ui.painter().text(
            egui::pos2(area.center().x, area.max.y - space::XXL),
            egui::Align2::CENTER_CENTER,
            s.call_alone,
            text::caption(),
            t.label_tertiary,
        );
    }

    let (columns, rows) = forced_columns
        .map(|columns| {
            let columns = columns.clamp(1, people.len());
            (columns, people.len().div_ceil(columns))
        })
        .unwrap_or_else(|| layout(people.len(), area));
    let gap = space::MD;
    let cell = Vec2::new(
        (area.width() - gap * (columns as f32 - 1.0)) / columns as f32,
        (area.height() - gap * (rows as f32 - 1.0)) / rows as f32,
    );
    let ctx = ui.ctx().clone();

    for (index, face) in people.iter().enumerate() {
        // Primeiro pega o quadro: a proporção do tile vem dele, não de uma
        // constante global. Uma call pode misturar 480×640 do Android com
        // 640×360 de uma webcam de desktop ao mesmo tempo.
        let texture = if face.camera {
            match (face.me, call.as_deref_mut()) {
                (true, Some(call)) => call.preview(&ctx),
                (false, Some(call)) => call.video(&ctx, &face.id, Kind::Camera),
                _ => None,
            }
        } else {
            None
        };
        let ratio = texture
            .as_ref()
            .map(|texture| texture.size_vec2())
            .filter(|size| size.x > 0.0 && size.y > 0.0)
            .map(|size| size.x / size.y)
            .unwrap_or(TILE_RATIO);

        let column = index % columns;
        let row = index / columns;
        let origin = egui::pos2(
            area.min.x + column as f32 * (cell.x + gap),
            area.min.y + row as f32 * (cell.y + gap),
        );
        let size = fit(cell, ratio);
        let rect = Rect::from_center_size(
            egui::pos2(origin.x + cell.x / 2.0, origin.y + cell.y / 2.0),
            size,
        );

        let crowded = face.camera
            && !face.me
            && call
                .as_deref()
                .is_some_and(|call| !call.has_slot(&face.id, Kind::Camera));

        tile(ui, state, store, t, s, rect, face, texture, crowded);
    }
}

/// Quantas colunas e linhas para `count` retratos, escolhendo a divisão que
/// aproveita melhor o espaço disponível.
fn layout(count: usize, area: Rect) -> (usize, usize) {
    let mut best = (count, 1);
    let mut score = f32::MIN;
    for columns in 1..=count {
        let rows = count.div_ceil(columns);
        let cell = Vec2::new(
            area.width() / columns as f32,
            area.height() / rows as f32,
        );
        let used = fit(cell, TILE_RATIO);
        let area_used = used.x * used.y * count as f32;
        if area_used > score {
            score = area_used;
            best = (columns, rows);
        }
    }
    best
}

/// O maior retângulo com a proporção pedida que cabe na célula, com uma
/// folga para o vão entre retratos.
fn fit(cell: Vec2, ratio: f32) -> Vec2 {
    let cell = cell - Vec2::splat(space::XS);
    if cell.x / cell.y > ratio {
        Vec2::new(cell.y * ratio, cell.y)
    } else {
        Vec2::new(cell.x, cell.x / ratio)
    }
}

/// Um retrato: o vídeo, ou a foto grande no meio de um cartão.
#[allow(clippy::too_many_arguments)]
fn tile(
    ui: &mut egui::Ui,
    state: &mut UiState,
    store: &Store,
    t: &Tokens,
    s: &Strings,
    rect: Rect,
    face: &Face,
    video: Option<egui::TextureHandle>,
    crowded: bool,
) {
    let corner = CornerRadius::same(radius::CARD);
    ui.painter().rect_filled(rect, corner, t.glass_opaque);

    match &video {
        Some(texture) => {
            // O retângulo já foi dimensionado com a proporção da própria
            // textura. Usa o quadro inteiro: nada de "cover" escondendo FOV
            // para forçar um Android 3:4 dentro de um tile 16:9.
            let mut mesh = egui::Mesh::with_texture(texture.id());
            mesh.add_rect_with_uv(
                rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            ui.painter()
                .with_clip_rect(rect)
                .add(egui::Shape::mesh(mesh));
        }
        None => {
            let ctx = ui.ctx().clone();
            let picture = state
                .media
                .avatar(&face.id, store.avatars.get(&face.id).map(String::as_str))
                .and_then(|texture| texture.frame(&ctx))
                .map(|handle| handle.id());
            let size = (rect.height() * 0.34).clamp(32.0, 96.0);
            let mut child = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(Rect::from_center_size(rect.center(), Vec2::splat(size)))
                    .layout(egui::Layout::centered_and_justified(
                        egui::Direction::TopDown,
                    )),
            );
            avatar(&mut child, t, &face.initials, size, face.tint, picture);
        }
    }

    if crowded {
        ui.painter().text(
            egui::pos2(rect.center().x, rect.center().y + rect.height() * 0.26),
            egui::Align2::CENTER_CENTER,
            s.call_no_slot,
            text::caption(),
            t.label_tertiary,
        );
    }

    // Quem está falando ganha o anel; é o que o olho procura numa grade.
    if face.speaking && !face.muted {
        ui.painter().rect_stroke(
            rect,
            corner,
            Stroke::new(2.0, t.online),
            egui::StrokeKind::Inside,
        );
    }

    // Nome e microfone no pé, sobre uma sombra curta para o texto sobreviver
    // a qualquer vídeo.
    let name = if face.me {
        format!("{} ({})", face.name, s.call_you)
    } else {
        face.name.clone()
    };
    let label = egui::pos2(rect.min.x + space::MD, rect.max.y - space::MD);
    let painter = ui.painter().with_clip_rect(rect);
    if video.is_some() {
        painter.rect_filled(
            Rect::from_min_max(
                egui::pos2(rect.min.x, rect.max.y - 28.0),
                rect.max,
            ),
            CornerRadius {
                nw: 0,
                ne: 0,
                sw: radius::CARD,
                se: radius::CARD,
            },
            Color32::from_black_alpha(110),
        );
    }
    let text_color = if video.is_some() {
        Color32::WHITE
    } else {
        t.label
    };
    let galley = painter.layout_no_wrap(name, text::caption(), text_color);
    let width = galley.size().x;
    painter.galley(
        egui::pos2(label.x, label.y - galley.size().y),
        galley,
        text_color,
    );
    if face.muted {
        painter.text(
            egui::pos2(label.x + width + space::SM, label.y - 7.0),
            egui::Align2::LEFT_CENTER,
            icon::MICROPHONE_SLASH,
            text::icon(12.0),
            if video.is_some() {
                Color32::WHITE
            } else {
                t.label_tertiary
            },
        );
    }
}

/// A fileira de controles embaixo da grade.
fn controls_row(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    rect: Rect,
    windowed: bool,
) {
    let size = 40.0;
    let mut buttons: Vec<(&str, &str, bool, bool, ChatAction)> = vec![
        (
            if store.call.muted {
                icon::MICROPHONE_SLASH
            } else {
                icon::MICROPHONE
            },
            if store.call.muted {
                s.call_mic_off
            } else {
                s.call_mic
            },
            !store.call.muted,
            false,
            ChatAction::ToggleMute,
        ),
        (
            if store.call.camera {
                icon::VIDEO_CAMERA
            } else {
                icon::VIDEO_CAMERA_SLASH
            },
            if store.call.camera {
                s.call_camera
            } else {
                s.call_camera_off
            },
            store.call.camera,
            false,
            ChatAction::ToggleCamera,
        ),
    ];
    if store.call.popped_out {
        buttons.push((
            icon::ARROW_SQUARE_IN,
            s.call_popin,
            false,
            false,
            ChatAction::PopOutCall(false),
        ));
    } else if !windowed {
        buttons.push((
            icon::ARROW_SQUARE_OUT,
            s.call_popout,
            false,
            false,
            ChatAction::PopOutCall(true),
        ));
    }
    buttons.push((
        icon::PHONE_X,
        s.call_leave,
        false,
        true,
        ChatAction::LeaveVoice,
    ));

    let total = buttons.len() as f32 * size + (buttons.len() as f32 - 1.0) * space::LG;
    let mut x = rect.center().x - total / 2.0 + size / 2.0;
    for (glyph, tip, on, danger, action) in buttons {
        let spot = Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::splat(size));
        if round_button(ui, t, spot, glyph, tip, on, danger) {
            state.actions.push(action);
        }
        x += size + space::LG;
    }
}

/// Botão redondo de controle. Ligado é a cor de destaque; sair é vermelho.
fn round_button(
    ui: &mut egui::Ui,
    t: &Tokens,
    rect: Rect,
    glyph: &str,
    tooltip: &str,
    on: bool,
    danger: bool,
) -> bool {
    let response = ui.interact(rect, egui::Id::new((glyph, rect.min.x as i32, rect.min.y as i32)), Sense::click());
    let hovered = response.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    let (fill, ink) = match (danger, on) {
        (true, _) => (t.danger, t.accent_label),
        (false, true) => (t.accent, t.accent_label),
        (false, false) => (t.fill_medium, t.label),
    };
    let fill = if hovered { fill } else { fill.gamma_multiply(0.85) };
    ui.painter().circle_filled(rect.center(), rect.width() / 2.0, fill);
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        text::icon(rect.width() * 0.44),
        ink,
    );
    response.on_hover_text(tooltip).clicked()
}

/// Onde a call é desenhada neste quadro, já contando com o que o usuário
/// pediu (encolher, janela própria).
pub fn stage_of(store: &Store) -> Option<Stage> {
    store.call.active().then(|| store.call.stage())
}
