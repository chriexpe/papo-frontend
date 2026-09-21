//! Peças visuais reutilizadas pelas telas.

use egui::{
    Color32, CornerRadius, FontId, Frame, Margin, Rect, Response, RichText, Sense, Stroke, Ui, Vec2,
};

use super::theme::{radius, space, text, Tokens, HIT_TARGET};

/// Moldura das colunas laterais.
///
/// Elas não têm conteúdo próprio por baixo e o KWin 6.7 não expõe mais o
/// protocolo de desfoque para clientes Wayland, então usam um tom opaco
/// próprio em vez de translucidez — o vidro fica por conta das pastilhas.
pub fn sidebar_frame(t: &Tokens) -> Frame {
    Frame::new().fill(t.glass_opaque).inner_margin(Margin::ZERO)
}

/// Cabeçalho de seção: maiúsculas pequenas, cor terciária.
pub fn section_caption(ui: &mut Ui, t: &Tokens, label: &str) {
    ui.add_space(space::LG);
    ui.label(
        RichText::new(label.to_uppercase())
            .font(text::caption())
            .color(t.label_tertiary),
    );
    ui.add_space(space::XS);
}

/// Avatar do usuário: a foto quando existe, senão as iniciais num círculo.
///
/// A foto é recortada pelo retângulo, igual à lista de membros e à pastilha
/// da conta — as três mostram a mesma imagem do mesmo jeito.
pub fn avatar(
    ui: &mut Ui,
    t: &Tokens,
    initials: &str,
    size: f32,
    tint: Option<Color32>,
    texture: Option<egui::TextureId>,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let base = tint.unwrap_or(t.accent);
    match texture {
        Some(texture) => {
            let mut mesh = egui::Mesh::with_texture(texture);
            mesh.add_rect_with_uv(
                rect,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
            ui.painter().with_clip_rect(rect).add(egui::Shape::mesh(mesh));
        }
        None => {
            let painter = ui.painter();
            painter.circle_filled(rect.center(), size / 2.0, base.gamma_multiply(0.30));
            painter.circle_stroke(
                rect.center(),
                size / 2.0 - 0.5,
                Stroke::new(1.0, base.gamma_multiply(0.55)),
            );
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                initials,
                FontId::new((size * 0.38).round(), egui::FontFamily::Name("semibold".into())),
                base,
            );
        }
    }
    response
}

/// Botão de ícone de 28×28 (alvo padrão do desktop nas HIG).
pub fn icon_button(ui: &mut Ui, t: &Tokens, icon: &str, tooltip: &str) -> Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(HIT_TARGET), Sense::click());
    let hovered = response.hovered();
    if hovered {
        ui.painter()
            .rect_filled(rect, CornerRadius::same(radius::FIELD), t.fill_soft);
    }
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        text::icon(15.0),
        if hovered { t.label } else { t.label_secondary },
    );
    response.on_hover_text(tooltip)
}


/// Superfície flutuante que mede o próprio conteúdo antes de fechar a
/// pastilha ao redor dele. Textos podem quebrar linha até `max_width`;
/// mensagens curtas não herdam uma largura fixa só porque outra é longa.
pub fn floating_pill(
    ctx: &egui::Context,
    id: egui::Id,
    t: &Tokens,
    translucent: bool,
    bottom_margin: f32,
    max_width: f32,
    contents: impl FnOnce(&mut Ui),
) {
    let safe = ctx.content_rect();
    let viewport = ctx.viewport_rect();
    let safe_bottom = (viewport.max.y - safe.max.y).max(0.0);
    let max_width = max_width
        .min((safe.width() - space::XL * 2.0).max(180.0))
        .max(180.0);

    egui::Area::new(id)
        .order(egui::Order::Foreground)
        .anchor(
            egui::Align2::CENTER_BOTTOM,
            Vec2::new(0.0, -(safe_bottom + bottom_margin)),
        )
        .constrain_to(safe)
        .show(ctx, |ui| {
            ui.set_max_width(max_width);
            Frame::new()
                .fill(t.pill_fill(translucent))
                .stroke(Stroke::new(1.0, t.separator))
                .corner_radius(CornerRadius::same(radius::SHEET))
                .inner_margin(Margin::symmetric(space::LG as i8, space::MD as i8))
                .show(ui, |ui| {
                    ui.set_max_width(max_width - space::LG * 2.0);
                    contents(ui);
                });
        });
}

/// Desvanecimento do conteúdo onde ele encontra uma barra (scroll edge effect).
pub fn scroll_edge_fade(ui: &Ui, rect: Rect, color: Color32, from_top: bool) {
    use egui::epaint::{Mesh, Vertex};

    let transparent = Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 0);
    let (top_color, bottom_color) = if from_top {
        (color, transparent)
    } else {
        (transparent, color)
    };

    let mut mesh = Mesh::default();
    let uv = egui::epaint::WHITE_UV;
    mesh.vertices.push(Vertex {
        pos: rect.left_top(),
        uv,
        color: top_color,
    });
    mesh.vertices.push(Vertex {
        pos: rect.right_top(),
        uv,
        color: top_color,
    });
    mesh.vertices.push(Vertex {
        pos: rect.right_bottom(),
        uv,
        color: bottom_color,
    });
    mesh.vertices.push(Vertex {
        pos: rect.left_bottom(),
        uv,
        color: bottom_color,
    });
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    ui.painter().add(egui::Shape::mesh(mesh));
}
