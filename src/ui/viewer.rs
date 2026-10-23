//! Visualizador em tela cheia: imagem com zoom e arraste, ou vídeo grande.
//!
//! Vive por cima de tudo, no mesmo quadro do egui: escurece o fundo, come os
//! cliques e devolve o que o usuário pediu.

use egui::{Color32, CornerRadius, Key, Rect, Sense, Stroke, Vec2};
use egui_phosphor::regular as icon;

use crate::api::models::{Attachment, Kind};
use crate::i18n::Strings;
use crate::media::{FileState, MediaStore, PlaySource};

use super::attachments::{clock, elide, size_label};
use super::theme::{radius, space, text, Tokens};

const ZOOM_MIN: f32 = 0.1;
const ZOOM_MAX: f32 = 12.0;

#[derive(Clone, Debug)]
pub struct Viewer {
    pub message_id: String,
    pub index: usize,
    /// Search results may reference messages outside the hydrated timeline.
    /// Keep their attachment snapshot with the viewer so image/video/audio
    /// still works without forcing a channel-history fetch first.
    attachments: Option<Vec<Attachment>>,
    /// External/embedded images use this very same viewer and image stage.
    external: Option<ExternalImage>,
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
            attachments: None,
            external: None,
            zoom: 1.0,
            offset: Vec2::ZERO,
            fitted: true,
        }
    }

    pub fn with_attachments(
        message_id: String,
        index: usize,
        attachments: Vec<Attachment>,
    ) -> Self {
        Self {
            message_id,
            index,
            attachments: Some(attachments),
            external: None,
            zoom: 1.0,
            offset: Vec2::ZERO,
            fitted: true,
        }
    }

    pub fn external_image(
        id: String,
        url: String,
        name: String,
        ephemeral: bool,
        favourite_id: Option<String>,
    ) -> Self {
        Self {
            message_id: String::new(),
            index: 0,
            attachments: None,
            external: Some(ExternalImage { id, url, name, ephemeral, favourite_id }),
            zoom: 1.0,
            offset: Vec2::ZERO,
            fitted: true,
        }
    }

    pub fn external_source(&self) -> Option<&ExternalImage> {
        self.external.as_ref()
    }

    pub fn source_attachments(&self) -> Option<&[Attachment]> {
        self.attachments.as_deref()
    }

    fn reset(&mut self) {
        self.zoom = 1.0;
        self.offset = Vec2::ZERO;
        self.fitted = true;
    }
}

#[derive(Clone, Debug)]
pub struct ExternalImage {
    pub id: String,
    pub url: String,
    pub name: String,
    pub ephemeral: bool,
    pub favourite_id: Option<String>,
}

pub enum ViewerAction {
    Close,
    Download { id: String, name: String },
    SaveCachedImage { path: std::path::PathBuf, name: String },
    ToggleFavourite(String),
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
    super::shell::claim_overlay_layer(ui.ctx(), layer, screen);
    ui.ctx().move_to_top(layer);
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

    let attachment = attachments.get(viewer.index);
    let external_owned = viewer.external.clone();
    let external = external_owned.as_ref();
    if attachment.is_none() && external.is_none() {
        return Some(ViewerAction::Close);
    }

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
    let mut action = if let Some(image) = viewer.external.as_ref() {
        let full = if let Some(embed_id) = image.id.strip_prefix("embed-image:") {
            media.preview_full(embed_id)
        } else if image.ephemeral {
            media.remote_ephemeral_full(&image.id, &image.url)
        } else {
            media.remote_image_full(&image.url)
        };
        let texture = full.and_then(|texture| texture.frame(ui.ctx())).cloned();
        stage_image_texture(ui, t, s, viewer, texture, stage, &mut content)
    } else if let Some(attachment) = attachment {
        if attachment.spoiler() && media.sensitive_hidden(&attachment.id) {
            let rect = Rect::from_center_size(
                stage.center(),
                Vec2::new(stage.width().min(320.0), 100.0),
            );
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(radius::CARD),
                t.fill_soft,
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                s.sensitive_reveal,
                text::body(),
                t.label_secondary,
            );
            content = rect;
            if ui.interact(rect, ui.id().with("reveal-viewer-spoiler"), Sense::click()).clicked() {
                media.reveal(&attachment.id);
            }
            None
        } else {
            match attachment.kind() {
                Kind::Video | Kind::Audio => {
                    stage_player(ui, t, s, media, attachment, stage, &mut content)
                }
                _ => stage_image(ui, t, s, media, viewer, attachment, stage, &mut content),
            }
        }
    } else {
        None
    };

    // Barra de cima: nome, contador e ações.
    let header = Rect::from_min_size(screen.min, Vec2::new(screen.width(), 56.0));
    ui.painter().text(
        egui::pos2(header.min.x + space::XXL, header.center().y),
        egui::Align2::LEFT_CENTER,
        elide(external.map_or_else(|| attachment.expect("attachment or external").name(), |image| image.name.as_str()), 60),
        text::headline(),
        Color32::WHITE,
    );
    let subtitle = if let Some(image) = external {
        image.url.clone()
    } else if let Some(attachment) = attachment {
        if attachments.len() > 1 {
            format!(
                "{} · {} / {}",
                size_label(attachment.size_bytes),
                viewer.index + 1,
                attachments.len()
            )
        } else {
            size_label(attachment.size_bytes)
        }
    } else {
        String::new()
    };
    ui.painter().text(
        egui::pos2(header.min.x + space::XXL, header.center().y + 16.0),
        egui::Align2::LEFT_CENTER,
        subtitle,
        text::footnote(),
        Color32::from_white_alpha(170),
    );

    let mut x = header.max.x - space::XXL - 16.0;
    let mut buttons = vec![(icon::X, "close")];
    if external.and_then(|image| image.favourite_id.as_ref()).is_some() {
        buttons.push((icon::STAR, "favourite"));
    }
    if external.is_none_or(|image| !image.ephemeral) {
        buttons.push((icon::DOWNLOAD_SIMPLE, "download"));
    }
    buttons.push((icon::ARROWS_IN, "fit"));
    for (glyph, tag) in buttons {
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
                    if let Some(image) = external {
                        let path = if let Some(embed_id) = image.id.strip_prefix("embed-image:") {
                            media.preview_full_cached_path(embed_id)
                        } else {
                            crate::media::remote_image_cached_path(&image.url)
                        };
                        if let Some(path) = path {
                            action = Some(ViewerAction::SaveCachedImage {
                                path,
                                name: image.name.clone(),
                            });
                        }
                    } else if let Some(attachment) = attachment {
                        action = Some(ViewerAction::Download {
                            id: attachment.id.clone(),
                            name: attachment.name().to_owned(),
                        });
                    }
                }
                "favourite" => {
                    if let Some(id) = external.and_then(|image| image.favourite_id.as_ref()) {
                        action = Some(ViewerAction::ToggleFavourite(id.clone()));
                    }
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

    stage_image_texture(ui, t, s, viewer, texture, stage, content)
}

/// Common image viewer for attachments, authenticated embeds and public GIFs.
/// This is deliberately the only image-stage implementation: zoom, pan,
/// double-click, fit and mouse/touch behavior stay identical for every source.
#[allow(clippy::too_many_arguments)]
fn stage_image_texture(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    viewer: &mut Viewer,
    texture: Option<egui::TextureHandle>,
    stage: Rect,
    content: &mut Rect,
) -> Option<ViewerAction> {
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
    // The stage owns drag/zoom input, so the backdrop never receives clicks
    // inside it. Treat a plain click on stage chrome outside the painted image
    // as a backdrop click as well.
    if response.clicked()
        && response
            .interact_pointer_pos()
            .is_some_and(|pointer| !rect.expand(space::MD).contains(pointer))
    {
        return Some(ViewerAction::Close);
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
    stage_playback(ui, t, media, &attachment.id, PlaySource::File(&path), video, stage, content)
}

/// O palco de tela cheia de um player, venha ele de anexo ou de link.
#[allow(clippy::too_many_arguments)]
pub(super) fn stage_playback(
    ui: &mut egui::Ui,
    t: &Tokens,
    media: &mut MediaStore,
    id: &str,
    source: PlaySource<'_>,
    video: bool,
    stage: Rect,
    content: &mut Rect,
) -> Option<ViewerAction> {
    let ctx = ui.ctx().clone();
    let player = media.start_source(id, source, video, &ctx)?;
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
        if let Some(player) = media.existing_player(id) {
            player.toggle();
        }
        media.solo(id);
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
        if let Some(player) = media.existing_player(id) {
            player.toggle();
        }
        media.solo(id);
    }

    // Alto-falante entre a linha do tempo e o relógio, igual ao do cartão.
    let speaker = Rect::from_center_size(
        egui::pos2(bar.max.x - 128.0, bar.center().y),
        Vec2::splat(24.0),
    );
    super::attachments::volume_control(ui, t, media, id, id, speaker);

    let line = Rect::from_min_max(
        egui::pos2(play.max.x + space::LG, bar.center().y - 3.0),
        egui::pos2(speaker.min.x - space::MD, bar.center().y + 3.0),
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
        if let Some(player) = media.existing_player(id) {
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


/// Fullscreen player for rich links. Images use Viewer::external_image
/// and draw() above; no duplicate image zoom/pan implementation remains.
#[allow(clippy::too_many_arguments)]
pub fn draw_remote_video(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    media: &mut MediaStore,
    id: &str,
    url: &str,
    name: &str,
    opened: f64,
) -> bool {
    let screen = ui.ctx().viewport_rect();
    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::new("papo-viewer"));
    super::shell::claim_overlay_layer(ui.ctx(), layer, screen);
    ui.ctx().move_to_top(layer);
    let mut top = ui.new_child(
        egui::UiBuilder::new()
            .layer_id(layer)
            .max_rect(screen)
            .sense(Sense::click_and_drag()),
    );
    top.painter()
        .rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(232));
    let background = top.interact(screen, top.id().with("viewer-backdrop"), Sense::click());

    if top.ctx().input(|input| input.key_pressed(Key::Escape)) {
        return true;
    }

    let stage = Rect::from_min_max(
        egui::pos2(screen.min.x + space::XXXL, screen.min.y + 64.0),
        egui::pos2(screen.max.x - space::XXXL, screen.max.y - 64.0),
    );
    let mut content = Rect::NOTHING;
    let source = if let Some(embed_id) = id.strip_prefix("embed-video:") {
        PlaySource::EmbedVideo(embed_id)
    } else {
        PlaySource::Remote(url)
    };
    let _ = stage_playback(
        &mut top, t, media, id, source, true, stage, &mut content,
    );
    if !content.is_positive() {
        top.painter().text(
            stage.center(),
            egui::Align2::CENTER_CENTER,
            s.downloading,
            text::body(),
            Color32::from_white_alpha(160),
        );
        top.ctx().request_repaint_after(std::time::Duration::from_millis(150));
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

    let close_rect = Rect::from_center_size(
        egui::pos2(header.max.x - space::XXL - 16.0, header.center().y),
        Vec2::splat(32.0),
    );
    let close = top
        .interact(close_rect, top.id().with("remote-viewer-close"), Sense::click())
        .on_hover_text(s.close);
    if close.hovered() {
        top.painter().rect_filled(
            close_rect,
            CornerRadius::same(radius::CONTROL),
            Color32::from_white_alpha(28),
        );
    }
    top.painter().text(
        close_rect.center(),
        egui::Align2::CENTER_CENTER,
        icon::X,
        text::icon(17.0),
        Color32::WHITE,
    );
    if close.clicked() {
        return true;
    }

    if background.clicked() && top.input(|input| input.time) > opened + 0.05 {
        let pointer = background.interact_pointer_pos().unwrap_or_default();
        let on_player = content.expand2(Vec2::new(space::MD, 55.0)).contains(pointer);
        let on_header = pointer.y < header.max.y;
        return !on_player && !on_header;
    }
    false
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
