//! Editor de recorte: "mover e ajustar" antes de subir foto, banner ou ícone.
//!
//! Qualquer imagem entra. A pessoa arrasta para posicionar e amplia com a
//! roda, a pinça ou a barra; a máscara mostra a forma onde a imagem vai
//! morar — círculo para a foto, retângulo 3:1 para o banner, squircle para o
//! ícone. No banner, a sua foto aparece por cima no lugar exato em que fica
//! no cartão de 300 pt, para dar para enquadrar em volta dela.
//!
//! Figurinha também passa por aqui: quadrada ou no formato original, com o
//! GIF animado tocando e, ao lado, como ela fica na conversa e no seletor.
//! O nome é digitado no próprio editor e Salvar já sobe.
//!
//! O recorte volta em frações do original; quem corta, reduz e converte é o
//! `media::prepare`, fora da thread da interface.

use std::path::PathBuf;

use egui::{Align2, Color32, CornerRadius, Id, Rect, Sense, Stroke, UiBuilder, Vec2};

use crate::i18n::Strings;
use crate::media::prepare::Crop;
use crate::platform::files::ImagePick;

use super::theme::{Tokens, radius, space, text};
use super::widgets::round_photo;

const SHEET_WIDTH: f32 = 460.0;
/// Figurinha: a coluna de prévias ao lado da tela de recorte.
const STICKER_SHEET_WIDTH: f32 = 640.0;
const SIDE_WIDTH: f32 = 200.0;
/// Abaixo disto a coluna não cabe e as prévias descem para baixo da tela.
const SIDE_MIN_SCREEN: f32 = 580.0;
const STACKED_EXTRA: f32 = 150.0;
const FOOTER: f32 = 46.0;
/// Tamanhos em que a figurinha aparece: sozinha na conversa (jumbo), no
/// meio do texto e no seletor.
const JUMBO: f32 = 34.0;
const INLINE: f32 = 18.0;
const PICKER: f32 = 28.0;
const CANVAS_HEIGHT: f32 = 300.0;
const HEADER: f32 = 52.0;
const MAX_ZOOM: f32 = 6.0;
/// A foto no cartão de 300 pt: 80 pt, a 16 pt da borda, invadindo o banner
/// em 44 pt. Os mesmos números de `profile.rs`.
const CARD_WIDTH: f32 = 300.0;
const CARD_AVATAR: f32 = 80.0;
const CARD_INSET: f32 = 16.0;
const CARD_OVERLAP: f32 = 44.0;

pub struct CropEditor {
    pub purpose: ImagePick,
    pub path: PathBuf,
    name: String,
    /// Tamanho do original, em pixels.
    size: Vec2,
    texture: egui::TextureHandle,
    /// GIF animado: cada quadro e quanto tempo fica; vazio se parado.
    frames: Vec<(egui::TextureHandle, f32)>,
    cycle: f32,
    zoom: f32,
    /// Centro do recorte, em pixels do original.
    center: egui::Pos2,
    /// Figurinha: recorte quadrado (padrão) ou no formato do original.
    square: bool,
    /// Figurinha: o nome, digitado aqui mesmo.
    sticker_name: String,
}

pub enum CropOutcome {
    Cancel,
    /// O recorte, e o nome quando é figurinha.
    Save(Crop, Option<String>),
}

/// Quem está editando: a foto aparece por cima do banner, e o nome na
/// prévia da conversa da figurinha.
pub struct Face {
    pub name: String,
    pub texture: Option<egui::TextureId>,
    pub initials: String,
    pub tint: Color32,
}

impl CropEditor {
    pub fn new(
        ctx: &egui::Context,
        purpose: ImagePick,
        path: PathBuf,
        name: String,
        size: [u32; 2],
        preview: egui::ColorImage,
        frames: Vec<(egui::ColorImage, f32)>,
    ) -> Self {
        let texture = ctx.load_texture("editor-de-recorte", preview, egui::TextureOptions::LINEAR);
        let frames: Vec<_> = frames
            .into_iter()
            .enumerate()
            .map(|(index, (image, delay))| {
                (
                    ctx.load_texture(format!("editor-de-recorte-{index}"), image, egui::TextureOptions::LINEAR),
                    delay,
                )
            })
            .collect();
        let cycle = frames.iter().map(|(_, delay)| delay).sum();
        let size = Vec2::new(size[0].max(1) as f32, size[1].max(1) as f32);
        // O nome do arquivo, sem extensão e só com o que um nome de
        // figurinha aceita, é um bom começo.
        let stem = std::path::Path::new(&name)
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        Self {
            purpose,
            path,
            name,
            size,
            texture,
            frames,
            cycle,
            zoom: 1.0,
            center: (size / 2.0).to_pos2(),
            square: true,
            sticker_name: sanitize(&stem),
        }
    }

    fn aspect(&self) -> f32 {
        if self.purpose == ImagePick::Sticker && !self.square {
            self.size.x / self.size.y
        } else {
            self.purpose.aspect().unwrap_or(1.0)
        }
    }

    fn animated(&self) -> bool {
        !self.frames.is_empty()
    }

    /// O quadro da vez; pede o próximo desenho para quando ele trocar.
    fn texture_now(&self, ctx: &egui::Context) -> egui::TextureId {
        if self.frames.is_empty() || self.cycle <= 0.0 {
            return self.texture.id();
        }
        let mut at = (ctx.input(|input| input.time) as f32) % self.cycle;
        for (texture, delay) in &self.frames {
            if at < *delay {
                ctx.request_repaint_after(std::time::Duration::from_secs_f32((delay - at).max(0.01)));
                return texture.id();
            }
            at -= delay;
        }
        self.frames[0].0.id()
    }

    /// Tamanho que o servidor vai receber, pela proporção do recorte.
    fn output_size(&self) -> (u32, u32) {
        let (max_w, max_h, _) = self.purpose.limits();
        let aspect = self.aspect();
        if aspect >= max_w as f32 / max_h as f32 {
            (max_w, (max_w as f32 / aspect).round() as u32)
        } else {
            ((max_h as f32 * aspect).round() as u32, max_h)
        }
    }

    /// Tamanho do recorte em pixels do original: o maior que cabe na
    /// proporção, dividido pelo zoom.
    fn crop_size(&self) -> Vec2 {
        let aspect = self.aspect();
        let base = if self.size.x / self.size.y > aspect {
            Vec2::new(self.size.y * aspect, self.size.y)
        } else {
            Vec2::new(self.size.x, self.size.x / aspect)
        };
        base / self.zoom
    }

    /// O recorte nunca sai da imagem.
    fn clamp(&mut self) {
        self.zoom = self.zoom.clamp(1.0, MAX_ZOOM);
        let half = self.crop_size() / 2.0;
        self.center.x = self.center.x.clamp(half.x, self.size.x - half.x);
        self.center.y = self.center.y.clamp(half.y, self.size.y - half.y);
    }

    pub fn crop(&self) -> Crop {
        let size = self.crop_size();
        let min = self.center - size / 2.0;
        Crop {
            x: min.x / self.size.x,
            y: min.y / self.size.y,
            width: size.x / self.size.x,
            height: size.y / self.size.y,
        }
    }
}

pub fn draw(
    ctx: &egui::Context,
    editor: &mut CropEditor,
    t: &Tokens,
    s: &Strings,
    face: &Face,
    back: bool,
) -> Option<CropOutcome> {
    let screen = ctx.content_rect();
    let sticker = editor.purpose == ImagePick::Sticker;
    let side = sticker && screen.width() >= SIDE_MIN_SCREEN;
    let base_width = if side { STICKER_SHEET_WIDTH } else { SHEET_WIDTH };
    let width = base_width.min(screen.width() - space::MD * 2.0);
    let stacked = if sticker && !side { STACKED_EXTRA } else { 0.0 };
    let canvas_h = CANVAS_HEIGHT.min((screen.height() - HEADER - 112.0 - stacked - space::XL).max(160.0));
    let height = HEADER + canvas_h + 112.0 + stacked;
    let sheet = Rect::from_center_size(screen.center(), Vec2::new(width, height));

    let layer = egui::LayerId::new(egui::Order::Foreground, Id::new("editor-de-recorte"));
    // Por cima da folha de ajustes, que também mora na camada Foreground.
    ctx.move_to_top(layer);
    egui::Area::new(layer.id)
        .order(egui::Order::Foreground)
        .fixed_pos(screen.min)
        .constrain(false)
        .show(ctx, |ui| {
            ui.set_min_size(screen.size());
            editor_contents(ui, ctx, editor, t, s, face, screen, sheet, width, canvas_h, back)
        })
        .inner
}

#[allow(clippy::too_many_arguments)]
fn editor_contents(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    editor: &mut CropEditor,
    t: &Tokens,
    s: &Strings,
    face: &Face,
    screen: Rect,
    sheet: Rect,
    width: f32,
    canvas_h: f32,
    back: bool,
) -> Option<CropOutcome> {
    // Véu e bloqueio: nada atrás recebe clique enquanto o editor está aberto.
    ui.painter()
        .rect_filled(screen, CornerRadius::ZERO, Color32::from_black_alpha(110));
    ui.interact(
        screen,
        Id::new("editor-de-recorte-bloqueio"),
        Sense::click_and_drag(),
    );

    let corners = CornerRadius::same(radius::SHEET);
    ui.painter()
        .add(ui.visuals().window_shadow.as_shape(sheet, corners));
    ui.painter().rect(
        sheet,
        corners,
        t.elevated_bg,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    // Cabeçalho: o que se está ajustando e de que arquivo.
    let title = match editor.purpose {
        ImagePick::Banner => s.crop_banner,
        ImagePick::ServerIcon => s.crop_icon,
        ImagePick::Sticker => s.sticker_new,
        ImagePick::ActivityArt => s.crop_activity,
        _ => s.crop_avatar,
    };
    let title_galley = ui
        .painter()
        .layout_no_wrap(title.to_owned(), text::title3(), t.label);
    let title_pos = egui::pos2(
        sheet.min.x + space::XL,
        sheet.min.y + HEADER / 2.0 - title_galley.size().y / 2.0,
    );
    let title_w = title_galley.size().x;
    ui.painter().galley(title_pos, title_galley, t.label);
    let meta = format!(
        "{} · {} × {}",
        editor.name, editor.size.x as u32, editor.size.y as u32
    );
    ui.painter()
        .with_clip_rect(Rect::from_x_y_ranges(
            title_pos.x + title_w + space::MD..=sheet.max.x - space::XL,
            sheet.y_range(),
        ))
        .text(
            egui::pos2(
                title_pos.x + title_w + space::MD,
                sheet.min.y + HEADER / 2.0,
            ),
            Align2::LEFT_CENTER,
            meta,
            text::callout(),
            t.label_tertiary,
        );
    ui.painter().line_segment(
        [
            egui::pos2(sheet.min.x, sheet.min.y + HEADER),
            egui::pos2(sheet.max.x, sheet.min.y + HEADER),
        ],
        Stroke::new(1.0, t.separator),
    );

    // --- Tela de recorte --------------------------------------------------
    let sticker = editor.purpose == ImagePick::Sticker;
    let side = sticker && width >= STICKER_SHEET_WIDTH - 1.0;
    let canvas_w = if side { width - SIDE_WIDTH } else { width };
    let canvas = Rect::from_min_size(
        egui::pos2(sheet.min.x, sheet.min.y + HEADER),
        Vec2::new(canvas_w, canvas_h),
    );
    let canvas_response = ui.interact(canvas, Id::new("editor-de-recorte-tela"), Sense::drag());
    let aspect = editor.aspect();
    let room = canvas.shrink(space::XXXL);
    let frame_w = room.width().min(room.height() * aspect);
    let frame = Rect::from_center_size(canvas.center(), Vec2::new(frame_w, frame_w / aspect));

    // Gestos: arrastar move, roda e pinça ampliam em torno do centro.
    let crop_w = editor.crop_size().x;
    let scale = frame.width() / crop_w;
    if canvas_response.dragged() {
        editor.center -= canvas_response.drag_delta() / scale;
    }
    // A roda é do editor inteiro enquanto ele está aberto: sobre a imagem
    // ela amplia, e em lugar nenhum ela rola a conversa lá atrás.
    let (scroll, touch) = ui.input_mut(|input| {
        let taken = (input.smooth_scroll_delta.y, input.multi_touch());
        input.smooth_scroll_delta = Vec2::ZERO;
        taken
    });
    if canvas_response.hovered() {
        editor.zoom *= (scroll * 0.0025).exp();
    }
    // Pinça (celular, trackpad): com dois dedos não há "hover" confiável,
    // e o editor é modal — o gesto vale onde quer que comece. Dois dedos
    // arrastando também movem a imagem.
    if let Some(touch) = touch {
        editor.zoom *= touch.zoom_delta;
        editor.center -= touch.translation_delta / scale;
    } else if canvas_response.hovered() {
        // Ctrl + roda e o gesto de pinça do trackpad chegam como zoom.
        editor.zoom *= ui.input(|input| input.zoom_delta());
    }
    if canvas_response.hovered() || canvas_response.dragged() {
        ui.ctx().set_cursor_icon(if canvas_response.dragged() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::Grab
        });
    }
    editor.clamp();

    let scale = frame.width() / editor.crop_size().x;
    let image_rect = Rect::from_min_size(
        frame.center() - editor.center.to_vec2() * scale,
        editor.size * scale,
    );
    let painter = ui.painter().with_clip_rect(canvas);
    painter.rect_filled(canvas, CornerRadius::ZERO, Color32::from_gray(12));
    let now_texture = editor.texture_now(ctx);
    painter.image(
        now_texture,
        image_rect,
        Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
        Color32::WHITE,
    );

    // Máscara: escurece tudo fora da forma. Fora do retângulo, quatro
    // faixas; dentro dele, um traço grosso que só sobra nos cantos que a
    // forma não cobre (círculo, squircle).
    let dim = Color32::from_black_alpha(150);
    for band in [
        Rect::from_min_max(canvas.min, egui::pos2(canvas.max.x, frame.min.y)),
        Rect::from_min_max(egui::pos2(canvas.min.x, frame.max.y), canvas.max),
        Rect::from_min_max(
            egui::pos2(canvas.min.x, frame.min.y),
            egui::pos2(frame.min.x, frame.max.y),
        ),
        Rect::from_min_max(
            egui::pos2(frame.max.x, frame.min.y),
            egui::pos2(canvas.max.x, frame.max.y),
        ),
    ] {
        painter.rect_filled(band, CornerRadius::ZERO, dim);
    }
    let shape_radius = match editor.purpose {
        ImagePick::Avatar => frame.width() / 2.0,
        ImagePick::ServerIcon => frame.width() * 0.22,
        ImagePick::Sticker => radius::CARD as f32,
        // O mesmo canto do quadro de 56 px no cartão de atividade.
        ImagePick::ActivityArt => frame.width() * radius::FIELD as f32 / 56.0,
        _ => radius::SHEET as f32,
    };
    let corner_band = frame.width();
    painter.with_clip_rect(frame).rect_stroke(
        frame.expand(corner_band / 2.0),
        CornerRadius::same((shape_radius + corner_band / 2.0).min(255.0) as u8),
        Stroke::new(corner_band, dim),
        egui::StrokeKind::Middle,
    );
    painter.rect_stroke(
        frame,
        CornerRadius::same(shape_radius.min(255.0) as u8),
        Stroke::new(1.0, Color32::from_white_alpha(220)),
        egui::StrokeKind::Middle,
    );

    // No banner, a foto por cima, onde ela fica no cartão.
    if editor.purpose == ImagePick::Banner {
        let k = frame.width() / CARD_WIDTH;
        let size = CARD_AVATAR * k;
        let avatar = Rect::from_min_size(
            egui::pos2(frame.min.x + CARD_INSET * k, frame.max.y - CARD_OVERLAP * k),
            Vec2::splat(size),
        );
        painter.circle_filled(avatar.center(), size / 2.0 + 4.0 * k, t.elevated_bg);
        match face.texture {
            Some(texture) => round_photo(&painter, avatar, texture, Color32::WHITE),
            None => {
                painter.circle_filled(avatar.center(), size / 2.0, face.tint.gamma_multiply(0.30));
                painter.text(
                    avatar.center(),
                    Align2::CENTER_CENTER,
                    &face.initials,
                    egui::FontId::new(
                        (size * 0.32).round(),
                        egui::FontFamily::Name("semibold".into()),
                    ),
                    face.tint,
                );
            }
        }
    }

    // --- Zoom, dica ------------------------------------------------------------
    let footer = Rect::from_min_max(egui::pos2(sheet.min.x, sheet.max.y - FOOTER), sheet.max);
    let controls = Rect::from_min_max(
        egui::pos2(canvas.min.x, canvas.max.y),
        egui::pos2(canvas.max.x, footer.min.y),
    )
    .shrink2(Vec2::new(space::XL, space::MD));
    let mut outcome = None;
    let crop = editor.crop();
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(controls)
            .layout(egui::Layout::top_down(egui::Align::Min)),
        |ui| {
            ui.spacing_mut().item_spacing.y = space::SM;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = space::MD;
                ui.label(
                    egui::RichText::new(egui_phosphor::regular::IMAGE)
                        .font(text::icon(12.0))
                        .color(t.label_secondary),
                );
                let toggle_w = if sticker { 150.0 } else { 0.0 };
                ui.spacing_mut().slider_width = (ui.available_width() - 26.0 - toggle_w).max(60.0);
                ui.add(egui::Slider::new(&mut editor.zoom, 1.0..=MAX_ZOOM).show_value(false));
                ui.label(
                    egui::RichText::new(egui_phosphor::regular::IMAGE)
                        .font(text::icon(18.0))
                        .color(t.label_secondary),
                );
                if sticker {
                    let mut square = editor.square;
                    if segmented(ui, t, &mut square, &[(true, s.crop_square), (false, s.crop_original)]) {
                        editor.square = square;
                        editor.zoom = 1.0;
                        editor.center = (editor.size / 2.0).to_pos2();
                    }
                }
            });
            let mut hint = s.crop_hint.to_owned();
            if editor.purpose == ImagePick::Banner {
                hint = format!("{hint} {}", s.crop_banner_hint);
            }
            if editor.animated() {
                hint = format!("{hint} {}", s.sticker_crop_hint);
            }
            ui.add(
                egui::Label::new(
                    egui::RichText::new(hint)
                        .font(text::footnote())
                        .color(t.label_tertiary),
                )
                .wrap(),
            );
            // Sem espaço para a coluna, nome e prévias descem para cá.
            if sticker && !side {
                ui.add_space(space::XS);
                sticker_side(ui, ctx, editor, t, s, face, now_texture, crop, false);
            }
        },
    );
    if side {
        let aside = Rect::from_min_max(
            egui::pos2(canvas.max.x, sheet.min.y + HEADER),
            egui::pos2(sheet.max.x, footer.min.y),
        );
        ui.painter().line_segment(
            [aside.left_top(), aside.left_bottom()],
            Stroke::new(1.0, t.separator),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(aside.shrink(space::LG))
                .layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| sticker_side(ui, ctx, editor, t, s, face, now_texture, crop, true),
        );
    }

    // --- Pé: o que sai, e os botões -----------------------------------------------
    let ready = !sticker || !editor.sticker_name.is_empty();
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(footer.shrink2(Vec2::new(space::XL, space::MD)))
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            let (w, h) = editor.output_size();
            let (_, _, bytes) = editor.purpose.limits();
            let weight = if bytes >= 1024 * 1024 {
                format!("{} {} MB", s.upto, bytes / (1024 * 1024))
            } else {
                format!("{} {} KB", s.upto, bytes / 1024)
            };
            let kind = if editor.animated() {
                format!("{} · {} {}", s.sticker_animated, editor.frames.len(), s.sticker_frames)
            } else {
                "WebP".to_owned()
            };
            ui.label(
                egui::RichText::new(format!("{w} × {h} · {kind} · {weight}"))
                    .font(text::footnote())
                    .color(t.label_tertiary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = space::SM;
                if button(ui, t, s.save, true, ready) {
                    outcome = Some(save(editor));
                }
                if button(ui, t, s.cancel, false, true) {
                    outcome = Some(CropOutcome::Cancel);
                }
            });
        },
    );
    editor.clamp();

    let (escape, enter) = ctx.input(|input| {
        (
            input.key_pressed(egui::Key::Escape),
            input.key_pressed(egui::Key::Enter),
        )
    });
    if escape || back {
        outcome = Some(CropOutcome::Cancel);
    } else if enter && ready {
        outcome = Some(save(editor));
    }
    outcome
}

fn save(editor: &CropEditor) -> CropOutcome {
    let name = (editor.purpose == ImagePick::Sticker).then(|| editor.sticker_name.clone());
    CropOutcome::Save(editor.crop(), name)
}

/// Nome de figurinha: o que cabe entre dois-pontos numa mensagem — letras,
/// números, `_` e `-` — e no máximo 32, o limite do servidor.
fn sanitize(raw: &str) -> String {
    raw.chars()
        .map(|c| if c == ' ' { '_' } else { c })
        .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
        .take(32)
        .collect()
}

/// Nome e prévias da figurinha: ao lado da tela de recorte (coluna) ou
/// embaixo dela, quando a tela é estreita.
#[allow(clippy::too_many_arguments)]
fn sticker_side(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    editor: &mut CropEditor,
    t: &Tokens,
    s: &Strings,
    face: &Face,
    texture: egui::TextureId,
    crop: Crop,
    column: bool,
) {
    ui.spacing_mut().item_spacing = Vec2::new(space::SM, space::SM);
    caption(ui, t, s.sticker_name_caption);
    name_field(ui, ctx, t, &mut editor.sticker_name);
    let uv = Rect::from_min_size(egui::pos2(crop.x, crop.y), Vec2::new(crop.width, crop.height));
    let aspect = editor.aspect();

    let chat = |ui: &mut egui::Ui| {
        // Sozinha numa mensagem: o tamanho grande.
        ui.horizontal(|ui| {
            let (face_rect, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
            match face.texture {
                Some(texture) => round_photo(ui.painter(), face_rect, texture, Color32::WHITE),
                None => {
                    ui.painter().circle_filled(face_rect.center(), 12.0, face.tint.gamma_multiply(0.30));
                    ui.painter().text(
                        face_rect.center(),
                        Align2::CENTER_CENTER,
                        &face.initials,
                        egui::FontId::new(9.0, egui::FontFamily::Name("semibold".into())),
                        face.tint,
                    );
                }
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = space::XXS;
                ui.label(egui::RichText::new(&face.name).font(text::headline()).color(t.label));
                sticker_image(ui, texture, uv, aspect, JUMBO);
            });
        });
        // No meio do texto.
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::XS;
            ui.add_space(24.0 + space::SM);
            ui.label(egui::RichText::new(s.sticker_inline_before).font(text::message()).color(t.label));
            sticker_image(ui, texture, uv, aspect, INLINE);
            ui.label(egui::RichText::new(s.sticker_inline_after).font(text::message()).color(t.label));
        });
    };
    let picker = |ui: &mut egui::Ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::XS;
            for index in 0..4 {
                let (tile, _) = ui.allocate_exact_size(Vec2::splat(PICKER + 8.0), Sense::hover());
                ui.painter().rect_filled(tile, CornerRadius::same(radius::FIELD), t.fill_soft);
                if index == 1 {
                    ui.painter().rect_stroke(
                        tile,
                        CornerRadius::same(radius::FIELD),
                        Stroke::new(2.0, t.accent),
                        egui::StrokeKind::Inside,
                    );
                    let inner = Rect::from_center_size(tile.center(), Vec2::splat(PICKER));
                    paint_sticker(ui.painter(), inner, texture, uv, aspect);
                }
            }
        });
    };

    if column {
        ui.add_space(space::MD);
        caption(ui, t, s.sticker_in_chat);
        chat(ui);
        ui.add_space(space::MD);
        caption(ui, t, s.sticker_in_picker);
        picker(ui);
    } else {
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                caption(ui, t, s.sticker_in_chat);
                chat(ui);
            });
            ui.add_space(space::LG);
            ui.vertical(|ui| {
                caption(ui, t, s.sticker_in_picker);
                picker(ui);
            });
        });
    }
}

fn caption(ui: &mut egui::Ui, t: &Tokens, label: &str) {
    ui.label(
        egui::RichText::new(label.to_uppercase())
            .font(text::caption())
            .color(t.label_tertiary),
    );
}

/// Campo do nome, com os dois-pontos em volta, como o nome aparece numa
/// mensagem.
fn name_field(ui: &mut egui::Ui, ctx: &egui::Context, t: &Tokens, value: &mut String) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(radius::FIELD), t.fill_soft);
    let colon = |x: f32| {
        ui.painter().text(
            egui::pos2(x, rect.center().y),
            Align2::CENTER_CENTER,
            ":",
            text::body(),
            t.label_tertiary,
        );
    };
    colon(rect.min.x + space::MD);
    colon(rect.max.x - space::MD);
    let inner = rect.shrink2(Vec2::new(space::MD + 6.0, 2.0));
    #[cfg(target_os = "android")]
    {
        let _ = crate::platform::native_field::show(
            ctx,
            "editor-de-recorte:nome",
            value,
            inner,
            "",
            crate::platform::native_field::Mode::Text,
            32,
            false,
            t.label,
            t.label_tertiary,
            text::body().size,
        );
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = ctx;
        let mut child = ui.new_child(UiBuilder::new().max_rect(inner));
        child.add_sized(
            inner.size(),
            egui::TextEdit::singleline(value)
                .char_limit(32)
                .frame(egui::Frame::NONE)
                .font(text::body())
                .vertical_align(egui::Align::Center),
        );
    }
    let clean = sanitize(value);
    if clean != *value {
        *value = clean;
    }
}

fn sticker_image(ui: &mut egui::Ui, texture: egui::TextureId, uv: Rect, aspect: f32, side: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
    paint_sticker(ui.painter(), rect, texture, uv, aspect);
}

/// A figurinha cabe no quadrado sem distorcer: no formato original ela
/// fica mais baixa ou mais estreita que ele.
fn paint_sticker(painter: &egui::Painter, rect: Rect, texture: egui::TextureId, uv: Rect, aspect: f32) {
    let size = if aspect >= 1.0 {
        Vec2::new(rect.width(), rect.width() / aspect)
    } else {
        Vec2::new(rect.height() * aspect, rect.height())
    };
    painter.image(texture, Rect::from_center_size(rect.center(), size), uv, Color32::WHITE);
}

/// Segmentado pequeno (Quadrado / Original). Devolve `true` quando muda.
fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, t: &Tokens, current: &mut T, options: &[(T, &str)]) -> bool {
    let height = 24.0;
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, label)| ui.painter().layout_no_wrap((*label).to_owned(), text::callout(), t.label).size().x + space::LG)
        .collect();
    let (rect, base) = ui.allocate_exact_size(Vec2::new(widths.iter().sum(), height), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(radius::CONTROL), t.fill_medium);
    let mut x = rect.min.x;
    let mut changed = false;
    for ((value, label), width) in options.iter().zip(widths) {
        let slot = Rect::from_min_size(egui::pos2(x, rect.min.y), Vec2::new(width, height));
        x += width;
        let selected = *current == *value;
        let response = ui.interact(slot, base.id.with(*label), Sense::click());
        if selected {
            ui.painter().rect_filled(slot.shrink(2.0), CornerRadius::same(radius::CONTROL - 1), t.elevated_bg);
        }
        ui.painter().text(
            slot.center(),
            Align2::CENTER_CENTER,
            *label,
            text::callout(),
            if selected { t.label } else { t.label_secondary },
        );
        if response.clicked() && !selected {
            *current = *value;
            changed = true;
        }
    }
    changed
}

fn button(ui: &mut egui::Ui, t: &Tokens, label: &str, primary: bool, enabled: bool) -> bool {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), text::callout(), t.label);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(galley.size().x + space::LG * 2.0, 26.0),
        Sense::click(),
    );
    let hovered = response.hovered() && enabled;
    let fill = match (primary, hovered) {
        (true, false) => t.accent.gamma_multiply(if enabled { 1.0 } else { 0.4 }),
        (true, true) => t.accent.gamma_multiply(0.85),
        (false, false) => t.fill_soft,
        (false, true) => t.fill_medium,
    };
    ui.painter()
        .rect_filled(rect, CornerRadius::same(radius::CONTROL), fill);
    ui.painter().galley(
        rect.center() - galley.size() / 2.0,
        galley,
        if primary { t.accent_label } else { t.label },
    );
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked() && enabled
}

#[cfg(test)]
mod tests {
    use super::*;

    fn editor(purpose: ImagePick, w: f32, h: f32) -> (CropEditor, egui::Context) {
        let ctx = egui::Context::default();
        let editor = CropEditor::new(
            &ctx,
            purpose,
            PathBuf::from("x.png"),
            "x.png".into(),
            [w as u32, h as u32],
            egui::ColorImage::new([2, 2], vec![Color32::WHITE; 4]),
            Vec::new(),
        );
        (editor, ctx)
    }

    #[test]
    fn banner_starts_as_the_widest_three_to_one_strip() {
        let (editor, _ctx) = editor(ImagePick::Banner, 4000.0, 3000.0);
        let crop = editor.crop();
        assert!((crop.width - 1.0).abs() < 1e-4);
        let pixels_h = crop.height * 3000.0;
        assert!((4000.0 / pixels_h - 3.0).abs() < 1e-3);
    }

    #[test]
    fn avatar_crop_is_square_and_stays_inside_after_a_wild_drag() {
        let (mut editor, _ctx) = editor(ImagePick::Avatar, 1200.0, 800.0);
        editor.zoom = 2.0;
        editor.center = egui::pos2(-5000.0, 99999.0);
        editor.clamp();
        let crop = editor.crop();
        assert!((crop.width * 1200.0 - crop.height * 800.0).abs() < 0.5);
        assert!(crop.x >= 0.0 && crop.y >= 0.0);
        assert!(crop.x + crop.width <= 1.0 + 1e-5 && crop.y + crop.height <= 1.0 + 1e-5);
    }

    #[test]
    fn sticker_original_keeps_the_image_shape() {
        let (mut editor, _ctx) = editor(ImagePick::Sticker, 480.0, 360.0);
        assert_eq!(editor.output_size(), (512, 512));
        editor.square = false;
        editor.clamp();
        assert_eq!(editor.output_size(), (512, 384));
        let crop = editor.crop();
        assert!((crop.width - 1.0).abs() < 1e-4 && (crop.height - 1.0).abs() < 1e-4);
    }

    #[test]
    fn sticker_names_only_keep_what_a_shortcode_can_hold() {
        assert_eq!(sanitize("meu gato: feliz!"), "meu_gato_feliz");
        assert_eq!(sanitize(&"a".repeat(40)).len(), 32);
    }

    #[test]
    fn zoom_is_bounded() {
        let (mut editor, _ctx) = editor(ImagePick::ServerIcon, 512.0, 512.0);
        editor.zoom = 100.0;
        editor.clamp();
        assert_eq!(editor.zoom, MAX_ZOOM);
        editor.zoom = 0.1;
        editor.clamp();
        assert_eq!(editor.zoom, 1.0);
    }
}
