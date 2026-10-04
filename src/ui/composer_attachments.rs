//! Anexos à espera de envio, dentro do composer: cada um vira um cartão com
//! prévia (imagem, capa de vídeo) e, empilhadas à direita dele, as ações —
//! remover, spoiler, anonimizar e editar em outra janela.
//!
//! O spoiler é aplicado pelo backend; aqui só se marca e se mostra como o
//! destinatário verá. Anonimizar roda no envio (ver `platform::anonymize`).

use egui::{
    Align, Color32, CornerRadius, Id, Layout, Rect, Sense, Stroke, UiBuilder, Vec2, pos2, vec2,
};
use egui_phosphor::regular as icon;

use papo_core::api::client::Upload;
use papo_core::api::models::Kind;

use crate::i18n::Strings;
use crate::media::MediaStore;

use super::attachments::{elide, size_label};
use super::theme::{Tokens, radius, space, text};
use super::widgets::photo;

/// Lado da prévia quadrada.
const TILE: f32 = 92.0;
/// Largura da cápsula de ações à direita da prévia.
const STACK_W: f32 = 24.0;
const BUTTON: f32 = 20.0;
/// Altura da faixa inteira no composer.
pub const BAND_H: f32 = TILE + space::XL;
/// Altura da faixa do cartão de link.
pub const LINK_H: f32 = 40.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileAction {
    Remove,
    Spoiler,
    Anonymize,
    /// Abre o arquivo no aplicativo padrão do sistema.
    EditExternally,
}

/// Desenha a faixa. Devolve a ação pedida no quadro, se houver.
pub fn band(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    media: &mut MediaStore,
    uploads: &[Upload],
    band: Rect,
) -> Option<(usize, TileAction)> {
    let mut requested = None;
    // O arquivo pode ser editado em outro programa; olhar de novo de vez em
    // quando é o que faz a prévia acompanhar.
    ui.ctx()
        .request_repaint_after(std::time::Duration::from_millis(1200));
    ui.scope_builder(
        UiBuilder::new()
            .max_rect(band.shrink2(vec2(space::LG, space::SM)))
            .layout(Layout::left_to_right(Align::Center)),
        |ui| {
            egui::ScrollArea::horizontal()
                .id_salt("composer-attach-scroll")
                .scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = space::MD;
                        for (index, upload) in uploads.iter().enumerate() {
                            if let Some(action) = tile(ui, t, s, media, upload, index) {
                                requested = Some((index, action));
                            }
                        }
                    });
                });
        },
    );
    requested
}

/// Recorte central que preenche um quadrado sem distorcer.
fn cover_uv(size: Vec2) -> Rect {
    if size.x <= 0.0 || size.y <= 0.0 {
        return super::widgets::FULL_UV;
    }
    let aspect = size.x / size.y;
    let span = if aspect > 1.0 {
        vec2(1.0 / aspect, 1.0)
    } else {
        vec2(1.0, aspect)
    };
    Rect::from_center_size(pos2(0.5, 0.5), span)
}

fn modified_stamp(upload: &Upload) -> u64 {
    std::fs::metadata(&upload.path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |elapsed| elapsed.as_millis() as u64)
}

fn tile(
    ui: &mut egui::Ui,
    t: &Tokens,
    s: &Strings,
    media: &mut MediaStore,
    upload: &Upload,
    index: usize,
) -> Option<TileAction> {
    let (rect, _) = ui.allocate_exact_size(vec2(TILE + space::SM + STACK_W, TILE), Sense::hover());
    let thumb = Rect::from_min_size(rect.min, Vec2::splat(TILE));
    let stack = Rect::from_min_max(pos2(thumb.max.x + space::SM, rect.min.y), rect.max);
    let corner = CornerRadius::same(radius::SHEET);
    let kind = Kind::guess(&upload.mime, &upload.name);

    ui.painter().rect_filled(thumb, corner, t.fill_soft);

    let texture = match kind {
        Kind::Image | Kind::Video => {
            let id = format!("{}#{}", upload.path.display(), modified_stamp(upload));
            media
                .local_preview(&id, &upload.path, kind == Kind::Video)
                .and_then(|texture| texture.frame(ui.ctx()))
                .cloned()
        }
        _ => None,
    };
    match &texture {
        Some(texture) => {
            let tint = if upload.spoiler {
                Color32::from_gray(70)
            } else {
                Color32::WHITE
            };
            photo(
                ui.painter(),
                thumb,
                texture.id(),
                cover_uv(texture.size_vec2()),
                corner,
                tint,
            );
        }
        None => {
            let glyph = match kind {
                Kind::Image => icon::IMAGE,
                Kind::Video => icon::FILM_STRIP,
                Kind::Audio => icon::MICROPHONE,
                Kind::Other => icon::FILE,
            };
            ui.painter().text(
                thumb.center() - vec2(0.0, 6.0),
                egui::Align2::CENTER_CENTER,
                glyph,
                text::icon(28.0),
                t.label_tertiary,
            );
        }
    }

    if kind == Kind::Video && !upload.spoiler {
        let badge = Rect::from_center_size(thumb.center(), Vec2::splat(30.0));
        ui.painter()
            .circle_filled(badge.center(), 15.0, Color32::from_black_alpha(130));
        ui.painter().text(
            badge.center(),
            egui::Align2::CENTER_CENTER,
            icon::PLAY,
            text::icon(14.0),
            Color32::WHITE,
        );
    }

    if upload.spoiler {
        ui.painter()
            .rect_filled(thumb, corner, Color32::from_black_alpha(120));
        ui.painter().text(
            thumb.center() - vec2(0.0, 8.0),
            egui::Align2::CENTER_CENTER,
            icon::EYE_SLASH,
            text::icon(22.0),
            Color32::from_white_alpha(220),
        );
        ui.painter().text(
            thumb.center() + vec2(0.0, 12.0),
            egui::Align2::CENTER_CENTER,
            s.spoiler_badge,
            text::footnote(),
            Color32::from_white_alpha(200),
        );
    }

    // Legenda no pé: nome (mascarado se for anonimizar) e tamanho.
    let caption_h = 28.0;
    let caption = Rect::from_min_max(pos2(thumb.min.x, thumb.max.y - caption_h), thumb.max);
    ui.painter().rect_filled(
        caption,
        CornerRadius {
            nw: 0,
            ne: 0,
            sw: radius::SHEET,
            se: radius::SHEET,
        },
        Color32::from_black_alpha(120),
    );
    let shown_name = if upload.anonymize {
        let extension = std::path::Path::new(&upload.name)
            .extension()
            .map(|ext| format!(".{}", ext.to_string_lossy()))
            .unwrap_or_default();
        format!("••••••{extension}")
    } else {
        elide(&upload.name, 14)
    };
    ui.painter().text(
        pos2(caption.min.x + space::SM, caption.center().y - 6.0),
        egui::Align2::LEFT_CENTER,
        shown_name,
        text::footnote(),
        Color32::WHITE,
    );
    ui.painter().text(
        pos2(caption.min.x + space::SM, caption.center().y + 6.0),
        egui::Align2::LEFT_CENTER,
        size_label(upload.size as i64),
        text::footnote(),
        Color32::from_white_alpha(170),
    );

    ui.painter().rect_stroke(
        thumb,
        corner,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );

    // Cápsula de vidro com as ações, empilhadas à direita.
    ui.painter().rect(
        stack,
        CornerRadius::same((STACK_W / 2.0) as u8),
        t.fill_soft,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    let actions: [(TileAction, &str, &str, bool); 4] = [
        (TileAction::Remove, icon::X, s.attach_remove, false),
        (
            TileAction::Spoiler,
            icon::EYE_SLASH,
            s.attach_spoiler,
            upload.spoiler,
        ),
        (
            TileAction::Anonymize,
            icon::SHIELD_CHECK,
            s.attach_anonymize,
            upload.anonymize,
        ),
        (
            TileAction::EditExternally,
            icon::PENCIL_SIMPLE,
            s.attach_edit_external,
            false,
        ),
    ];
    let gap = (stack.height() - BUTTON * actions.len() as f32) / (actions.len() as f32 + 1.0);
    let mut requested = None;
    for (slot, (action, glyph, tooltip, active)) in actions.into_iter().enumerate() {
        let button = Rect::from_center_size(
            pos2(
                stack.center().x,
                stack.min.y + gap + BUTTON / 2.0 + slot as f32 * (BUTTON + gap),
            ),
            Vec2::splat(BUTTON),
        );
        let response = ui.interact(
            button,
            Id::new(("composer-attach", index, slot)),
            Sense::click(),
        );
        let danger = action == TileAction::Remove;
        if active {
            ui.painter()
                .circle_filled(button.center(), BUTTON / 2.0, t.accent.gamma_multiply(0.9));
        } else if response.hovered() {
            ui.painter().circle_filled(
                button.center(),
                BUTTON / 2.0,
                if danger {
                    t.danger.gamma_multiply(0.25)
                } else {
                    t.fill_medium
                },
            );
        }
        ui.painter().text(
            button.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            text::icon(12.0),
            if active {
                t.accent_label
            } else if response.hovered() && danger {
                t.danger
            } else if response.hovered() {
                t.label
            } else {
                t.label_secondary
            },
        );
        if response.on_hover_text(tooltip).clicked() {
            requested = Some(action);
        }
    }
    requested
}

/// Primeiro endereço http(s) do texto, sem pontuação colada no fim.
pub fn first_url(text: &str) -> Option<&str> {
    let start = text.match_indices("http").map(|(at, _)| at).find(|at| {
        let rest = &text[*at..];
        (rest.starts_with("https://") || rest.starts_with("http://"))
            && text[..*at]
                .chars()
                .next_back()
                .is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '<'))
    })?;
    let rest = &text[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let url = rest[..end].trim_end_matches(['.', ',', ';', ':', '!', '?', ')', ']', '>']);
    let host_part = url.split("://").nth(1)?;
    (!host_part.is_empty()).then_some(url)
}

/// `(host, resto do caminho)` para o cartão de link.
pub fn url_parts(url: &str) -> (&str, &str) {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let rest = rest.strip_prefix("www.").unwrap_or(rest);
    match rest.find(['/', '?', '#']) {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, ""),
    }
}

/// Cartão básico do link que está sendo digitado: host e caminho. O preview
/// rico (título, imagem) só existe depois do envio, feito pelo servidor.
pub fn link_card(ui: &mut egui::Ui, t: &Tokens, url: &str, band: Rect) {
    let (host, path) = url_parts(url);
    let card = band.shrink2(vec2(space::LG, space::XS));
    ui.painter().rect(
        card,
        CornerRadius::same(radius::CARD),
        t.fill_soft,
        Stroke::new(1.0, t.separator),
        egui::StrokeKind::Inside,
    );
    ui.painter().rect_filled(
        Rect::from_min_size(card.min, vec2(3.0, card.height())),
        CornerRadius::same(2),
        t.accent,
    );
    ui.painter().text(
        pos2(card.min.x + space::LG + 2.0, card.center().y),
        egui::Align2::LEFT_CENTER,
        icon::LINK_SIMPLE,
        text::icon(15.0),
        t.accent,
    );
    let x = card.min.x + space::LG + 26.0;
    ui.painter().text(
        pos2(x, card.center().y - 6.0),
        egui::Align2::LEFT_CENTER,
        elide(host, 40),
        text::caption(),
        t.label,
    );
    if !path.is_empty() {
        ui.painter().text(
            pos2(x, card.center().y + 7.0),
            egui::Align2::LEFT_CENTER,
            elide(path, 56),
            text::footnote(),
            t.label_tertiary,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_first_url_and_trims_punctuation() {
        assert_eq!(
            first_url("olha isso https://exemplo.com/a?b=1, legal"),
            Some("https://exemplo.com/a?b=1")
        );
        assert_eq!(first_url("(http://x.org)"), Some("http://x.org"));
        assert_eq!(first_url("https://"), None);
        assert_eq!(first_url("sem link"), None);
        assert_eq!(first_url("xhttp://colado.com"), None);
    }

    #[test]
    fn splits_host_and_path() {
        assert_eq!(
            url_parts("https://www.site.com/a/b?q=1"),
            ("site.com", "/a/b?q=1")
        );
        assert_eq!(url_parts("http://site.com"), ("site.com", ""));
    }
}
