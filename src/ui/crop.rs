//! Editor de recorte: "mover e ajustar" antes de subir foto, banner ou ícone.
//!
//! Qualquer imagem entra. A pessoa arrasta para posicionar e amplia com a
//! roda, a pinça ou a barra; a máscara mostra a forma onde a imagem vai
//! morar — círculo para a foto, retângulo 3:1 para o banner, squircle para o
//! ícone. No banner, a sua foto aparece por cima no lugar exato em que fica
//! no cartão de 300 pt, para dar para enquadrar em volta dela.
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
    zoom: f32,
    /// Centro do recorte, em pixels do original.
    center: egui::Pos2,
}

pub enum CropOutcome {
    Cancel,
    Save(Crop),
}

/// Quem está editando: a foto aparece por cima do banner.
pub struct Face {
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
    ) -> Self {
        let texture = ctx.load_texture("editor-de-recorte", preview, egui::TextureOptions::LINEAR);
        let size = Vec2::new(size[0].max(1) as f32, size[1].max(1) as f32);
        Self {
            purpose,
            path,
            name,
            size,
            texture,
            zoom: 1.0,
            center: (size / 2.0).to_pos2(),
        }
    }

    fn aspect(&self) -> f32 {
        self.purpose.aspect().unwrap_or(1.0)
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
    let width = SHEET_WIDTH.min(screen.width() - space::MD * 2.0);
    let canvas_h = CANVAS_HEIGHT.min(screen.height() * 0.5);
    let height = HEADER + canvas_h + 112.0;
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
    let canvas = Rect::from_min_size(
        egui::pos2(sheet.min.x, sheet.min.y + HEADER),
        Vec2::new(width, canvas_h),
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
    painter.image(
        editor.texture.id(),
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

    // --- Zoom, dica, botões -------------------------------------------------
    let controls = Rect::from_min_max(egui::pos2(sheet.min.x, canvas.max.y), sheet.max)
        .shrink2(Vec2::new(space::XL, space::MD));
    let mut outcome = None;
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
                ui.spacing_mut().slider_width = ui.available_width() - 26.0;
                ui.add(egui::Slider::new(&mut editor.zoom, 1.0..=MAX_ZOOM).show_value(false));
                ui.label(
                    egui::RichText::new(egui_phosphor::regular::IMAGE)
                        .font(text::icon(18.0))
                        .color(t.label_secondary),
                );
            });
            let hint = if editor.purpose == ImagePick::Banner {
                format!("{} {}", s.crop_hint, s.crop_banner_hint)
            } else {
                s.crop_hint.to_owned()
            };
            ui.add(
                egui::Label::new(
                    egui::RichText::new(hint)
                        .font(text::footnote())
                        .color(t.label_tertiary),
                )
                .wrap(),
            );
            ui.add_space(space::XS);
            ui.horizontal(|ui| {
                let (w, h, bytes) = editor.purpose.limits();
                ui.label(
                    egui::RichText::new(format!("{w} × {h} · até {} MB", bytes / (1024 * 1024)))
                        .font(text::footnote())
                        .color(t.label_tertiary),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = space::SM;
                    if button(ui, t, s.save, true) {
                        outcome = Some(CropOutcome::Save(editor.crop()));
                    }
                    if button(ui, t, s.cancel, false) {
                        outcome = Some(CropOutcome::Cancel);
                    }
                });
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
    } else if enter {
        outcome = Some(CropOutcome::Save(editor.crop()));
    }
    outcome
}

fn button(ui: &mut egui::Ui, t: &Tokens, label: &str, primary: bool) -> bool {
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), text::callout(), t.label);
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(galley.size().x + space::LG * 2.0, 26.0),
        Sense::click(),
    );
    let fill = match (primary, response.hovered()) {
        (true, false) => t.accent,
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
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response.clicked()
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
