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

/// Proporção de cada retrato da grade. 16:9 é o que a câmera entrega.
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
                tint: person.and_then(|person| person.role_color),
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
        let ring = avatar(&mut child, t, &initials, 18.0, person.and_then(|p| p.role_color), texture);
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
    let painter = ui.painter();
    painter.rect(
        rect,
        CornerRadius::same(radius::CARD),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

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

/// O canal de voz visto de fora: quem está lá e o convite para entrar.
pub fn lobby(ui: &mut egui::Ui, store: &Store, state: &mut UiState, t: &Tokens, s: &Strings) {
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, t.content_bg);

    let channel_id = store.selected_channel.clone();
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
        if people == 0 {
            s.call_empty_room.to_owned()
        } else {
            format!("{people} {}", s.call_in_channel)
        },
        text::body(),
        t.label_secondary,
    );

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
        s.call_join,
        text::body(),
        t.accent_label,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        state.actions.push(ChatAction::JoinVoice(channel_id));
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
        (icon::X, s.call_collapse, ChatAction::CollapseCall(true)),
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

/// A pastilha da folha encolhida: a call continua, fora do caminho.
pub fn pill(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    t: &Tokens,
    s: &Strings,
    area: Rect,
) {
    let width = 236.0;
    let rect = Rect::from_min_size(
        egui::pos2(area.center().x - width / 2.0, area.min.y + space::LG),
        Vec2::new(width, 36.0),
    );
    ui.painter().rect(
        rect,
        CornerRadius::same(18),
        t.pill_fill(state.translucent),
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    let name = store
        .channel(&store.call.channel_id)
        .map(|channel| channel.name.clone())
        .unwrap_or_default();
    ui.painter().circle_filled(
        egui::pos2(rect.min.x + space::LG, rect.center().y),
        3.5,
        t.online,
    );
    ui.painter().text(
        egui::pos2(rect.min.x + space::LG + 10.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        format!("{name} · {}", store.call.members().len()),
        text::caption(),
        t.label,
    );

    let mut x = rect.max.x - space::MD - 13.0;
    for (glyph, tip, on, danger, action) in [
        (icon::PHONE_X, s.call_leave, false, true, ChatAction::LeaveVoice),
        (
            icon::ARROWS_OUT,
            s.call_expand,
            false,
            false,
            ChatAction::CollapseCall(false),
        ),
        (
            if store.call.muted {
                icon::MICROPHONE_SLASH
            } else {
                icon::MICROPHONE
            },
            s.call_mic,
            !store.call.muted,
            false,
            ChatAction::ToggleMute,
        ),
    ] {
        let spot = Rect::from_center_size(egui::pos2(x, rect.center().y), Vec2::splat(26.0));
        if round_button(ui, t, spot, glyph, tip, on, danger) {
            state.actions.push(action);
        }
        x -= 26.0 + space::XXS;
    }
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

/// A grade: uma pessoa por retrato, vídeo quando há, foto quando não há.
fn grid(
    ui: &mut egui::Ui,
    store: &Store,
    state: &mut UiState,
    mut call: Option<&mut Call>,
    t: &Tokens,
    s: &Strings,
    area: Rect,
) {
    let people = faces(store);
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

    let (columns, rows) = layout(people.len(), area);
    let gap = space::MD;
    let cell = Vec2::new(
        (area.width() - gap * (columns as f32 - 1.0)) / columns as f32,
        (area.height() - gap * (rows as f32 - 1.0)) / rows as f32,
    );
    let ctx = ui.ctx().clone();

    for (index, face) in people.iter().enumerate() {
        let column = index % columns;
        let row = index / columns;
        let origin = egui::pos2(
            area.min.x + column as f32 * (cell.x + gap),
            area.min.y + row as f32 * (cell.y + gap),
        );
        // O retrato mantém a proporção da câmera dentro da célula, centrado:
        // a grade fica regular mesmo com uma pessoa só.
        let size = fit(cell, TILE_RATIO);
        let rect = Rect::from_center_size(
            egui::pos2(origin.x + cell.x / 2.0, origin.y + cell.y / 2.0),
            size,
        );

        let texture = if face.camera {
            match (face.me, call.as_deref_mut()) {
                (true, Some(call)) => call.preview(&ctx),
                (false, Some(call)) => call.video(&ctx, &face.id, Kind::Camera),
                _ => None,
            }
        } else {
            None
        };
        // Câmera ligada e nenhum lugar livre: o retrato diz por que o vídeo
        // não aparece, em vez de ficar parecendo uma câmera que travou.
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
            // O vídeo cobre o retrato: sobra é cortada, não esticada.
            let mut mesh = egui::Mesh::with_texture(texture.id());
            mesh.add_rect_with_uv(
                rect,
                cover(texture.size_vec2(), rect.size()),
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

/// O retângulo de textura que cobre o destino sem deformar a imagem.
fn cover(texture: Vec2, target: Vec2) -> Rect {
    if texture.x <= 0.0 || texture.y <= 0.0 || target.y <= 0.0 {
        return Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    }
    let source = texture.x / texture.y;
    let wanted = target.x / target.y;
    let (w, h) = if source > wanted {
        (wanted / source, 1.0)
    } else {
        (1.0, source / wanted)
    };
    Rect::from_center_size(egui::pos2(0.5, 0.5), Vec2::new(w, h))
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
