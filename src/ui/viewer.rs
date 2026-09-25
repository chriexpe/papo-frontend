//! Visualizador em tela cheia: imagem com zoom e arraste, ou vídeo grande.
//!
//! Vive por cima de tudo, no mesmo quadro do egui: escurece o fundo, come os
//! cliques e devolve o que o usuário pediu.

use egui::{Color32, CornerRadius, Key, Rect, Sense, Stroke, Vec2};
use egui_phosphor::regular as icon;

use crate::api::models::{Attachment, Kind};
use crate::i18n::Strings;
use crate::media::{FileState, MediaStore};

use super::attachments::{clock, elide, size_label};
use super::theme::{radius, space, text, Tokens};

const ZOOM_MIN: f32 = 0.1;
const ZOOM_MAX: f32 = 12.0;

#[derive(Clone, Debug)]
pub struct Viewer {
    pub message_id: String,
    pub index: usize,
    zoom: f32,
    offset: Vec2,
    /// Sem zoom manual: a imagem acompanha a janela.
    fitted: bool,
}

impl Viewer {
    pub fn new(message_id: String, index: usize) -> Self {
        Self {
            message_id,
            index,
            zoom: 1.0,
            offset: Vec2::ZERO,
            fitted: true,
        }
    }

    fn reset(&mut self) {
        self.zoom = 1.0;
        self.offset = Vec2::ZERO;
        self.fitted = true;
    }
}

pub enum ViewerAction {
    Close,
    Download { id: String, name: String },
}

/// Desenha o visualizador. Devolve `None` quando nada mudou.
pub fn draw(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    media: &mut MediaStore,
    viewer: &mut Viewer,
    attachments: &[Attachment],
) -> Option<ViewerAction> {
    let screen = ui.ctx().viewport_rect();
    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("papo-viewer"));
    let mut ui = ui.new_child(
        egui::UiBuilder::new()
            .layer_id(layer)
            .max_rect(screen)
            .sense(Sense::click_and_drag()),
    );
    let ui = &mut ui;

    ui.painter()
        .rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(232));
    // Register the backdrop first. Header buttons, stage controls and arrows
    // are added afterwards and therefore own overlapping clicks.
    let background = ui.interact(screen, ui.id().with("viewer-backdrop"), Sense::click());

    let Some(attachment) = attachments.get(viewer.index) else {
        return Some(ViewerAction::Close);
    };

    // Navegação e saída pelo teclado.
    let (close, next, previous, reset) = ui.ctx().input(|input| {
        (
            input.key_pressed(Key::Escape),
            input.key_pressed(Key::ArrowRight),
            input.key_pressed(Key::ArrowLeft),
            input.key_pressed(Key::Num0),
        )
    });
    if close {
        return Some(ViewerAction::Close);
    }
    if next && viewer.index + 1 < attachments.len() {
        viewer.index += 1;
        viewer.reset();
    }
    if previous && viewer.index > 0 {
        viewer.index -= 1;
        viewer.reset();
    }
    if reset {
        viewer.reset();
    }

    let stage = Rect::from_min_max(
        egui::pos2(screen.min.x + space::XXXL, screen.min.y + 64.0),
        egui::pos2(screen.max.x - space::XXXL, screen.max.y - 64.0),
    );

    // Onde a mídia foi desenhada: clicar fora disso fecha.
    let mut content = Rect::NOTHING;
    let mut action = match attachment.kind() {
        Kind::Video | Kind::Audio => {
            stage_player(ui, t, s, media, attachment, stage, &mut content)
        }
        _ => stage_image(ui, t, s, media, viewer, attachment, stage, &mut content),
    };

    // Barra de cima: nome, contador e ações.
    let header = Rect::from_min_size(screen.min, Vec2::new(screen.width(), 56.0));
    ui.painter().text(
        egui::pos2(header.min.x + space::XXL, header.center().y),
        egui::Align2::LEFT_CENTER,
        elide(attachment.name(), 60),
        text::headline(),
        Color32::WHITE,
    );
    let subtitle = if attachments.len() > 1 {
        format!(
            "{} · {} / {}",
            size_label(attachment.size_bytes),
            viewer.index + 1,
            attachments.len()
        )
    } else {
        size_label(attachment.size_bytes)
    };
    ui.painter().text(
        egui::pos2(header.min.x + space::XXL, header.center().y + 16.0),
        egui::Align2::LEFT_CENTER,
        subtitle,
        text::footnote(),
        Color32::from_white_alpha(170),
    );

    let mut x = header.max.x - space::XXL - 16.0;
    for (glyph, tag) in [
        (icon::X, "close"),
        (icon::DOWNLOAD_SIMPLE, "download"),
        (icon::ARROWS_IN, "fit"),
    ] {
        let rect = Rect::from_center_size(egui::pos2(x, header.center().y), Vec2::splat(32.0));
        let response = ui
            .interact(rect, ui.id().with(("viewer", tag)), Sense::click())
            .on_hover_text(match tag {
                "close" => s.close,
                "download" => s.viewer_download,
                _ => s.viewer_fit,
            });
        if response.hovered() {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(radius::CONTROL),
                Color32::from_white_alpha(28),
            );
        }
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            text::icon(17.0),
            Color32::WHITE,
        );
        if response.clicked() {
            match tag {
                "close" => action = Some(ViewerAction::Close),
                "download" => {
                    action = Some(ViewerAction::Download {
                        id: attachment.id.clone(),
                        name: attachment.name().to_owned(),
                    })
                }
                _ => viewer.reset(),
            }
        }
        x -= 38.0;
    }

    // Setas laterais quando a mensagem tem mais de um anexo.
    if attachments.len() > 1 {
        if viewer.index > 0
            && arrow(ui, t, egui::pos2(screen.min.x + 28.0, screen.center().y), icon::CARET_LEFT)
        {
            viewer.index -= 1;
            viewer.reset();
        }
        if viewer.index + 1 < attachments.len()
            && arrow(ui, t, egui::pos2(screen.max.x - 28.0, screen.center().y), icon::CARET_RIGHT)
        {
            viewer.index += 1;
            viewer.reset();
        }
    }

    // Clique no vazio ao redor da mídia fecha, como em qualquer visualizador.
    if background.clicked() && action.is_none() {
        let pointer = background.interact_pointer_pos().unwrap_or_default();
        let on_content = content.expand(space::MD).contains(pointer);
        let on_header = pointer.y < screen.min.y + 56.0;
        let on_controls = pointer.y > screen.max.y - 40.0;
        if !on_content && !on_header && !on_controls {
            action = Some(ViewerAction::Close);
        }
    }

    action
}

#[allow(clippy::too_many_arguments)]
fn stage_image(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    media: &mut MediaStore,
    viewer: &mut Viewer,
    attachment: &Attachment,
    stage: Rect,
    content: &mut Rect,
) -> Option<ViewerAction> {
    // A imagem cheia chega depois; até lá a miniatura segura o lugar.
    let full = media
        .full(&attachment.id)
        .and_then(|texture| texture.frame(ui.ctx()))
        .cloned();
    let texture = match full {
        Some(texture) => Some(texture),
        None => media
            .thumb(attachment)
            .and_then(|texture| texture.frame(ui.ctx()))
            .cloned(),
    };

    let Some(texture) = texture else {
        ui.painter().text(
            stage.center(),
            egui::Align2::CENTER_CENTER,
            s.downloading,
            text::body(),
            Color32::from_white_alpha(160),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(200));
        return None;
    };

    let natural = texture.size_vec2();
    let base = (stage.width() / natural.x)
        .min(stage.height() / natural.y)
        .min(1.0);
    let scale = if viewer.fitted { base } else { viewer.zoom };

    let response = ui.interact(
        stage,
        ui.id().with("viewer-stage"),
        Sense::click_and_drag(),
    );

    // Pinça de dois dedos. No toque não existe roda do mouse, e sem isto a
    // imagem não tinha como crescer no celular. O egui entrega o gesto já
    // pronto como fator — 1.0 quer dizer que ninguém beliscou — e o mesmo
    // valor chega do Ctrl+roda na área de trabalho, de graça.
    let pinch = ui.ctx().input(|input| input.zoom_delta());
    if (pinch - 1.0).abs() > 0.001 {
        let previous = scale;
        let next = (previous * pinch).clamp(ZOOM_MIN, ZOOM_MAX);
        // A âncora é o meio entre os dois dedos: a imagem cresce de onde se
        // está olhando, não do centro da tela.
        let anchor = ui
            .ctx()
            .input(|input| input.multi_touch().map(|touch| touch.center_pos))
            .or_else(|| ui.ctx().pointer_latest_pos());
        if let Some(anchor) = anchor {
            let centre = stage.center() + viewer.offset;
            let to_anchor = anchor - centre;
            viewer.offset -= to_anchor * (next / previous - 1.0);
        }
        viewer.zoom = next;
        viewer.fitted = false;
    }

    // Roda do mouse: zoom em torno do ponteiro.
    let scroll = ui.ctx().input(|input| input.smooth_scroll_delta.y);
    if response.hovered() && scroll.abs() > 0.1 {
        let previous = scale;
        let next = (previous * (1.0 + scroll * 0.0015)).clamp(ZOOM_MIN, ZOOM_MAX);
        if let Some(pointer) = ui.ctx().pointer_latest_pos() {
            let centre = stage.center() + viewer.offset;
            let to_pointer = pointer - centre;
            viewer.offset -= to_pointer * (next / previous - 1.0);
        }
        viewer.zoom = next;
        viewer.fitted = false;
    }
    if response.dragged() {
        viewer.offset += response.drag_delta();
        if viewer.fitted {
            viewer.zoom = base;
            viewer.fitted = false;
        }
    }
    if response.double_clicked() {
        if viewer.fitted {
            viewer.zoom = 1.0;
            viewer.fitted = false;
            viewer.offset = Vec2::ZERO;
        } else {
            viewer.reset();
        }
    }

    let size = natural * scale;
    let rect = Rect::from_center_size(stage.center() + viewer.offset, size);
    *content = rect;
    ui.painter().image(
        texture.id(),
        rect,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );

    // Nível de zoom, discreto, só quando foge do ajuste automático.
    if !viewer.fitted {
        let label = format!("{:.0}%", scale * 100.0);
        let badge = Rect::from_center_size(
            egui::pos2(stage.center().x, stage.max.y + 26.0),
            Vec2::new(64.0, 24.0),
        );
        ui.painter().rect_filled(
            badge,
            CornerRadius::same(radius::CONTROL),
            Color32::from_black_alpha(140),
        );
        ui.painter().text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            label,
            text::footnote(),
            Color32::WHITE,
        );
    }
    let _ = t;
    None
}

#[allow(clippy::too_many_arguments)]
fn stage_player(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    media: &mut MediaStore,
    attachment: &Attachment,
    stage: Rect,
    content: &mut Rect,
) -> Option<ViewerAction> {
    let state = media.file(&attachment.id, attachment.name());
    let FileState::Ready(path) = state else {
        ui.painter().text(
            stage.center(),
            egui::Align2::CENTER_CENTER,
            if matches!(state, FileState::Loading) {
                s.downloading
            } else {
                s.media_failed
            },
            text::body(),
            Color32::from_white_alpha(160),
        );
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(250));
        return None;
    };

    let video = matches!(attachment.kind(), Kind::Video);
    let ctx = ui.ctx().clone();
    let player = media.start_player(&attachment.id, &path, video, &ctx)?;
    let playing = player.is_playing();
    let position = player.position();
    let duration = player.duration();
    let aspect = player.aspect().clamp(0.3, 4.0);

    let frame_rect = if video {
        let mut size = Vec2::new(stage.width(), stage.width() / aspect);
        if size.y > stage.height() {
            size = Vec2::new(stage.height() * aspect, stage.height());
        }
        Rect::from_center_size(stage.center(), size)
    } else {
        Rect::from_center_size(stage.center(), Vec2::new(stage.width().min(520.0), 120.0))
    };

    *content = frame_rect;
    if video {
        if let Some(texture) = player.frame(&ctx) {
            ui.painter().image(
                texture.id(),
                frame_rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        } else {
            ui.painter()
                .rect_filled(frame_rect, CornerRadius::same(radius::CARD), Color32::BLACK);
        }
    } else {
        ui.painter().rect(
            frame_rect,
            CornerRadius::same(radius::CARD),
            Color32::from_white_alpha(12),
            Stroke::new(1.0, Color32::from_white_alpha(30)),
            egui::StrokeKind::Inside,
        );
        ui.painter().text(
            frame_rect.center() - Vec2::new(0.0, 16.0),
            egui::Align2::CENTER_CENTER,
            icon::MUSIC_NOTES,
            text::icon(26.0),
            Color32::from_white_alpha(170),
        );
    }

    // Um clique no vídeo alterna play/pause, como se espera.
    let surface = ui.interact(frame_rect, ui.id().with("viewer-video"), Sense::click());
    if surface.clicked() {
        if let Some(player) = media.existing_player(&attachment.id) {
            player.toggle();
        }
        media.solo(&attachment.id);
    }
    if !playing && video {
        ui.painter()
            .circle_filled(frame_rect.center(), 34.0, Color32::from_black_alpha(140));
        ui.painter().text(
            frame_rect.center(),
            egui::Align2::CENTER_CENTER,
            icon::PLAY,
            text::icon(26.0),
            Color32::WHITE,
        );
    }

    // Controles embaixo do palco.
    let bar = Rect::from_min_size(
        egui::pos2(frame_rect.min.x, frame_rect.max.y + space::MD),
        Vec2::new(frame_rect.width(), 40.0),
    );
    ui.painter().rect_filled(
        bar,
        CornerRadius::same(radius::CARD),
        Color32::from_black_alpha(160),
    );

    let play = Rect::from_center_size(
        egui::pos2(bar.min.x + space::XXL, bar.center().y),
        Vec2::splat(28.0),
    );
    let play_response = ui.interact(play, ui.id().with("viewer-play"), Sense::click());
    ui.painter().text(
        play.center(),
        egui::Align2::CENTER_CENTER,
        if playing { icon::PAUSE } else { icon::PLAY },
        text::icon(16.0),
        Color32::WHITE,
    );
    if play_response.clicked() {
        if let Some(player) = media.existing_player(&attachment.id) {
            player.toggle();
        }
        media.solo(&attachment.id);
    }

    let line = Rect::from_min_max(
        egui::pos2(play.max.x + space::LG, bar.center().y - 3.0),
        egui::pos2(bar.max.x - 110.0, bar.center().y + 3.0),
    );
    let progress = if duration > 0.0 {
        (position / duration).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    ui.painter()
        .rect_filled(line, CornerRadius::same(3), Color32::from_white_alpha(45));
    let played = Rect::from_min_size(line.min, Vec2::new(line.width() * progress, line.height()));
    ui.painter()
        .rect_filled(played, CornerRadius::same(3), t.accent);
    ui.painter()
        .circle_filled(egui::pos2(played.max.x, line.center().y), 6.0, Color32::WHITE);

    let seek = ui.interact(
        line.expand2(Vec2::new(0.0, 10.0)),
        ui.id().with("viewer-seek"),
        Sense::click_and_drag(),
    );
    if let Some(pointer) = seek
        .interact_pointer_pos()
        .filter(|_| seek.dragged() || seek.clicked())
    {
        let ratio = ((pointer.x - line.min.x) / line.width()).clamp(0.0, 1.0) as f64;
        if let Some(player) = media.existing_player(&attachment.id) {
            let duration = player.duration();
            player.seek(duration * ratio);
        }
    }

    ui.painter().text(
        egui::pos2(bar.max.x - space::XXL, bar.center().y),
        egui::Align2::RIGHT_CENTER,
        format!("{} / {}", clock(position), clock(duration)),
        text::footnote(),
        Color32::from_white_alpha(200),
    );

    if playing {
        ui.ctx().request_repaint();
    }
    None
}


pub enum RemoteViewerAction {
    Close,
    Download { path: std::path::PathBuf, name: String },
}

/// Same fullscreen image UI used for attachment images, backed by a public
/// rich-preview image instead of a server attachment.
#[allow(clippy::too_many_arguments)]
pub fn draw_remote_image(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    media: &mut MediaStore,
    id: &str,
    url: &str,
    name: &str,
    opened: f64,
    zoom: &mut f32,
    offset: &mut Vec2,
    fitted: &mut bool,
) -> Option<RemoteViewerAction> {
    let screen = ui.ctx().viewport_rect();
    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("papo-viewer"));
    let top = ui.new_child(
        egui::UiBuilder::new()
            .layer_id(layer)
            .max_rect(screen)
            .sense(Sense::click_and_drag()),
    );

    top.painter()
        .rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(232));
    let background =
        top.interact(screen, top.id().with("viewer-backdrop"), Sense::click());

    if top.ctx().input(|input| input.key_pressed(Key::Escape)) {
        return Some(RemoteViewerAction::Close);
    }

    let stage = Rect::from_min_max(
        egui::pos2(screen.min.x + space::XXXL, screen.min.y + 64.0),
        egui::pos2(screen.max.x - space::XXXL, screen.max.y - 64.0),
    );
    let texture = media
        .remote_image(id, url)
        .and_then(|texture| texture.frame(top.ctx()))
        .cloned();

    let mut content = Rect::NOTHING;
    if let Some(texture) = texture {
        let natural = texture.size_vec2();
        let base = (stage.width() / natural.x)
            .min(stage.height() / natural.y)
            .min(1.0);
        let scale = if *fitted { base } else { *zoom };
        let response = top.interact(
            stage,
            top.id().with("viewer-stage"),
            Sense::click_and_drag(),
        );

        let pinch = top.ctx().input(|input| input.zoom_delta());
        if (pinch - 1.0).abs() > 0.001 {
            let previous = scale;
            let next = (previous * pinch).clamp(ZOOM_MIN, ZOOM_MAX);
            let anchor = top
                .ctx()
                .input(|input| input.multi_touch().map(|touch| touch.center_pos))
                .or_else(|| top.ctx().pointer_latest_pos());
            if let Some(anchor) = anchor {
                let centre = stage.center() + *offset;
                *offset -= (anchor - centre) * (next / previous - 1.0);
            }
            *zoom = next;
            *fitted = false;
        }

        let scroll = top.ctx().input(|input| input.smooth_scroll_delta.y);
        if response.hovered() && scroll.abs() > 0.1 {
            let previous = scale;
            let next = (previous * (1.0 + scroll * 0.0015)).clamp(ZOOM_MIN, ZOOM_MAX);
            if let Some(pointer) = top.ctx().pointer_latest_pos() {
                let centre = stage.center() + *offset;
                *offset -= (pointer - centre) * (next / previous - 1.0);
            }
            *zoom = next;
            *fitted = false;
        }
        if response.dragged() {
            *offset += response.drag_delta();
            if *fitted {
                *zoom = base;
                *fitted = false;
            }
        }
        if response.double_clicked() {
            if *fitted {
                *zoom = 1.0;
                *fitted = false;
                *offset = Vec2::ZERO;
            } else {
                *zoom = 1.0;
                *offset = Vec2::ZERO;
                *fitted = true;
            }
        }

        let size = natural * scale;
        content = Rect::from_center_size(stage.center() + *offset, size);
        top.painter().image(
            texture.id(),
            content,
            Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
    } else {
        top.painter().text(
            stage.center(),
            egui::Align2::CENTER_CENTER,
            s.downloading,
            text::body(),
            Color32::from_white_alpha(160),
        );
        top.ctx()
            .request_repaint_after(std::time::Duration::from_millis(150));
    }

    let header = Rect::from_min_size(screen.min, Vec2::new(screen.width(), 56.0));
    top.painter().text(
        egui::pos2(header.min.x + space::XXL, header.center().y),
        egui::Align2::LEFT_CENTER,
        elide(name, 60),
        text::headline(),
        Color32::WHITE,
    );
    top.painter().text(
        egui::pos2(header.min.x + space::XXL, header.center().y + 16.0),
        egui::Align2::LEFT_CENTER,
        url,
        text::footnote(),
        Color32::from_white_alpha(170),
    );

    let mut action = None;
    let mut x = header.max.x - space::XXL - 16.0;
    for (glyph, tag) in [
        (icon::X, "close"),
        (icon::DOWNLOAD_SIMPLE, "download"),
        (icon::ARROWS_IN, "fit"),
    ] {
        let rect = Rect::from_center_size(egui::pos2(x, header.center().y), Vec2::splat(32.0));
        let response = top
            .interact(rect, top.id().with(("remote-viewer", tag)), Sense::click())
            .on_hover_text(match tag {
                "close" => s.close,
                "download" => s.viewer_download,
                _ => s.viewer_fit,
            });
        if response.hovered() {
            top.painter().rect_filled(
                rect,
                CornerRadius::same(radius::CONTROL),
                Color32::from_white_alpha(28),
            );
        }
        top.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            text::icon(17.0),
            Color32::WHITE,
        );
        if response.clicked() {
            match tag {
                "close" => action = Some(RemoteViewerAction::Close),
                "download" => {
                    if let Some(path) = crate::media::remote_image_cached_path(url) {
                        action = Some(RemoteViewerAction::Download {
                            path,
                            name: name.to_owned(),
                        });
                    }
                }
                _ => {
                    *zoom = 1.0;
                    *offset = Vec2::ZERO;
                    *fitted = true;
                }
            }
        }
        x -= 38.0;
    }

    if background.clicked() && action.is_none() {
        let now = top.input(|input| input.time);
        let pointer = background.interact_pointer_pos().unwrap_or_default();
        let on_content = content.expand(space::MD).contains(pointer);
        let on_header = pointer.y < screen.min.y + 56.0;
        if now > opened + 0.05 && !on_content && !on_header {
            action = Some(RemoteViewerAction::Close);
        }
    }

    action
}

fn arrow(ui: &mut egui::Ui, t: &Tokens, centre: egui::Pos2, glyph: &str) -> bool {
    let _ = t;
    let rect = Rect::from_center_size(centre, Vec2::splat(40.0));
    let response = ui.interact(rect, ui.id().with(("viewer-arrow", glyph)), Sense::click());
    let alpha = if response.hovered() { 40 } else { 20 };
    ui.painter().circle_filled(
        centre,
        20.0,
        Color32::from_white_alpha(alpha),
    );
    ui.painter().text(
        centre,
        egui::Align2::CENTER_CENTER,
        glyph,
        text::icon(18.0),
        Color32::WHITE,
    );
    response.clicked()
}
