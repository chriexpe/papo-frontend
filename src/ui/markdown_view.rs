//! Desenho da árvore de `markdown`.
//!
//! O texto segue o mesmo caminho de `rich_body`: cada palavra é um widget
//! (para o clique em endereços), emoji viram imagem e o aviso de link externo
//! é o de sempre. Em cima disso entram o estilo (negrito, itálico, riscado,
//! código), as menções clicáveis e os blocos (lista, citação, código, tabela).
//! Medidas em `em` seguem o CSS de `FormattedMessage.svelte` do cliente web.

use egui::{
    Align2, Color32, CornerRadius, FontFamily, FontId, Margin, RichText, Sense, Stroke, StrokeKind,
    Vec2,
};
use egui_phosphor::regular as icon;

use crate::i18n::Strings;
use crate::state::{Emoji, Store};

use super::emoji;
use super::markdown::{Block, ColumnAlign, Inline, ListItem, Style};
use super::shell::{UiState, muted_link_color};
use super::theme::{Appearance, Tokens, space, text};

/// O que todo desenho precisa saber, para não passar meia dúzia de argumentos
/// em cada chamada.
pub struct View<'a> {
    pub t: &'a Tokens,
    pub s: &'a Strings,
    pub store: &'a Store,
    pub color: Color32,
    pub edited: bool,
    /// Pode buscar imagens remotas agora (a linha está perto da tela).
    pub network: bool,
    /// Etiquetas (sem o `@`) que ganham pastilha no texto: `everyone` e
    /// `todos` quando quem escreveu pode usá-los, e os cargos de quem lê.
    /// Da maior para a menor.
    pub tags: &'a [String],
}

/// Altura do espaçador de linha em branco, em `em`.
const SPACER_EM: f32 = 1.55;
/// Tamanho do emoji dentro do texto, em pontos, na escala 1.
const EMOJI_SIZE: f32 = 18.0;

fn em() -> f32 {
    text::message().size
}

fn font(scale: f32, bold: bool) -> FontId {
    let base = text::message();
    let family = if bold {
        FontFamily::Name("semibold".into())
    } else {
        base.family
    };
    FontId::new(base.size * scale, family)
}

fn mono(scale: f32) -> FontId {
    let base = text::mono();
    FontId::new(base.size * scale, base.family)
}

/// Cor das bordas finas (citação, tabela, régua).
fn line_color(t: &Tokens) -> Color32 {
    t.label_tertiary
}

/// Desenha a mensagem inteira, na largura da coluna.
pub fn draw(ui: &mut egui::Ui, state: &mut UiState, v: &View, blocks: &[Block], width: f32) {
    ui.set_max_width(width);
    ui.scope(|ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        // Uma palavra que não cabe na coluna (célula estreita de tabela, por
        // exemplo) passa dela por até um espaço. Recortar na largura da coluna
        // garante que nada apareça fora da margem.
        let column = ui.max_rect();
        let clip = ui.clip_rect();
        ui.set_clip_rect(egui::Rect::from_min_max(
            egui::pos2(column.min.x.max(clip.min.x), clip.min.y),
            egui::pos2(column.max.x.min(clip.max.x), clip.max.y),
        ));
        let suffix = v.edited.then(|| format!("  ({})", v.s.edited));
        // O "(editado)" fica na última linha de texto, como no caminho simples.
        let inline_last = matches!(
            blocks.last(),
            Some(Block::Paragraph(_) | Block::Heading(..))
        );
        let inline_suffix = if inline_last { suffix.as_deref() } else { None };
        draw_blocks(ui, state, v, blocks, inline_suffix);
        if !inline_last && let Some(suffix) = &suffix {
            ui.label(
                RichText::new(suffix.trim_start())
                    .font(text::footnote())
                    .color(v.t.label_tertiary),
            );
        }
    });
}

fn draw_blocks(
    ui: &mut egui::Ui,
    state: &mut UiState,
    v: &View,
    blocks: &[Block],
    suffix: Option<&str>,
) {
    let last = blocks.len().saturating_sub(1);
    for (index, block) in blocks.iter().enumerate() {
        let suffix = if index == last { suffix } else { None };
        match block {
            Block::Paragraph(inlines) => draw_inlines(ui, state, v, inlines, 1.0, false, suffix),
            Block::Heading(level, inlines) => {
                let scale = match level {
                    1 => 1.6,
                    2 => 1.4,
                    3 => 1.2,
                    _ => 1.0,
                };
                ui.add_space(0.2 * em());
                draw_inlines(ui, state, v, inlines, scale, true, suffix);
                ui.add_space(0.2 * em());
            }
            Block::Quote(inner) => quote(ui, state, v, inner),
            Block::List { start, items } => list(ui, state, v, *start, items),
            Block::Code { lang, text } => code_block(ui, v, lang.as_deref(), text),
            Block::Rule => {
                ui.add_space(0.3 * em());
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
                ui.painter().hline(
                    rect.x_range(),
                    rect.center().y,
                    Stroke::new(1.0, line_color(v.t)),
                );
                ui.add_space(0.3 * em());
            }
            Block::Table { aligns, head, rows } => table(ui, state, v, aligns, head, rows),
            Block::Spacer => {
                ui.add_space(SPACER_EM * em());
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Texto em linha
// ---------------------------------------------------------------------------

fn draw_inlines(
    ui: &mut egui::Ui,
    state: &mut UiState,
    v: &View,
    items: &[Inline],
    scale: f32,
    bold: bool,
    suffix: Option<&str>,
) {
    if items.is_empty() && suffix.is_none() {
        return;
    }
    // A largura da coluna, medida antes de entrar na linha que quebra: lá
    // dentro `available_width` é só o que sobra da linha.
    let column = ui.available_width();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(0.0, space::XXS);
        let line_height = ui.fonts_mut(|fonts| fonts.row_height(&font(scale, bold)));
        let mut row_empty = true;
        for item in items {
            match item {
                Inline::Break => {
                    // Uma linha vazia precisa de altura própria.
                    if row_empty {
                        ui.allocate_exact_size(Vec2::new(0.0, line_height), Sense::hover());
                    }
                    ui.end_row();
                    row_empty = true;
                    continue;
                }
                Inline::Text { text, style, link } => {
                    text_run(ui, state, v, text, *style, link.as_deref(), scale, bold)
                }
                Inline::Code(code) => code_chip(ui, v, code, scale, column),
                Inline::Image { src, alt } => {
                    remote_image(ui, state, v, src, alt, column);
                    ui.end_row();
                    row_empty = true;
                    continue;
                }
                Inline::Mention { user_id, label } => {
                    mention_pill(ui, state, v, user_id, label, scale)
                }
            }
            row_empty = false;
        }
        if let Some(suffix) = suffix {
            ui.label(
                RichText::new(suffix)
                    .font(text::footnote())
                    .color(v.t.label_tertiary),
            );
        }
    });
}

fn styled(word: &str, font: FontId, color: Color32, style: Style) -> RichText {
    let mut rich = RichText::new(word).font(font).color(color);
    if style.italic {
        rich = rich.italics();
    }
    if style.strike {
        rich = rich.strikethrough();
    }
    rich
}

#[allow(clippy::too_many_arguments)]
fn text_run(
    ui: &mut egui::Ui,
    state: &mut UiState,
    v: &View,
    text: &str,
    style: Style,
    link: Option<&str>,
    scale: f32,
    bold: bool,
) {
    let font = font(scale, style.bold || bold);
    for token in emoji::tokenize(text, &v.store.emojis) {
        match token {
            emoji::Token::Text(chunk) => {
                for (segment, is_tag) in split_tags(&chunk, v.tags, link.is_some()) {
                    if is_tag {
                        tag_pill(ui, v, segment, scale);
                        continue;
                    }
                    for word in segment.split_inclusive(' ') {
                        if word.trim().is_empty() && word != " " {
                            continue;
                        }
                        let visible = word.trim_end();
                        let trailing = &word[visible.len()..];
                        // Dentro de um link o destino é o do link; fora, o
                        // endereço solto na própria palavra.
                        let target = match link {
                            Some(href) => Some(href.to_owned()),
                            None => papo_core::preview::extract_https_urls(visible)
                                .into_iter()
                                .next(),
                        };
                        let Some(target) = target else {
                            ui.label(styled(word, font.clone(), v.color, style));
                            continue;
                        };
                        let label = styled(visible, font.clone(), muted_link_color(ui, v.t), style)
                            .underline();
                        // http(s) passa pelo aviso de link externo; `mailto:` abre
                        // o app de e-mail direto. `#` e `/` aparecem como link mas
                        // não levam a lugar nenhum aqui.
                        let web = target.starts_with("http://") || target.starts_with("https://");
                        let mail = target.to_ascii_lowercase().starts_with("mailto:");
                        if web || mail {
                            let response = ui.add(egui::Label::new(label).sense(Sense::click()));
                            if response.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                            }
                            if response.clicked() {
                                if web {
                                    state.request_external_url(ui.ctx(), target);
                                } else {
                                    crate::platform::links::open_url(&target);
                                }
                            }
                        } else {
                            ui.label(label);
                        }
                        if !trailing.is_empty() {
                            ui.label(styled(trailing, font.clone(), v.color, style));
                        }
                    }
                }
            }
            emoji::Token::Unicode(glyph) => {
                let (rect, _) =
                    ui.allocate_exact_size(Vec2::splat(EMOJI_SIZE * scale), Sense::hover());
                emoji::draw_unicode(ui, v.t, &mut state.media, &glyph, rect);
            }
            emoji::Token::Custom(id) => {
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::splat(EMOJI_SIZE * scale), Sense::hover());
                emoji::draw_reaction(
                    ui,
                    v.t,
                    &mut state.media,
                    v.store,
                    &Emoji::Custom(id.clone()),
                    rect,
                );
                if let Some(custom) = v.store.emojis.iter().find(|emoji| emoji.id == id) {
                    response.on_hover_text(format!(":{}:", custom.name));
                }
            }
        }
    }
}

/// Divide o texto em trechos comuns e etiquetas (`@everyone`, `@Cargo`), na
/// ordem em que aparecem. Dentro de um link nada vira etiqueta.
fn split_tags<'a>(chunk: &'a str, tags: &[String], in_link: bool) -> Vec<(&'a str, bool)> {
    if in_link || tags.is_empty() {
        return vec![(chunk, false)];
    }
    let mut out = Vec::new();
    let mut last = 0;
    for span in papo_core::notification::tag_spans(chunk, tags) {
        if span.start > last {
            out.push((&chunk[last..span.start], false));
        }
        out.push((&chunk[span.clone()], true));
        last = span.end;
    }
    if last < chunk.len() {
        out.push((&chunk[last..], false));
    }
    if out.is_empty() {
        out.push((chunk, false));
    }
    out
}

/// Etiqueta que chama quem lê (ou todos): a mesma pastilha da menção, sem
/// clique.
fn tag_pill(ui: &mut egui::Ui, v: &View, word: &str, scale: f32) {
    let size = em() * scale;
    let pad = Vec2::new(0.3 * size, 0.06 * size);
    let color = ui.visuals().hyperlink_color;
    let galley = ui
        .painter()
        .layout_no_wrap(word.to_owned(), font(scale, true), color);
    let (rect, _) = ui.allocate_exact_size(galley.size() + pad * 2.0, Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(5), v.t.accent.gamma_multiply(0.16));
    ui.painter().galley(rect.min + pad, galley, color);
}

/// Imagem remota de `![alt](url)`. Só busca quando a linha está perto da tela
/// e o endereço passa na checagem de segurança da mídia; enquanto não chega, um
/// quadro com o texto alternativo. O toque abre o endereço (com o aviso).
fn remote_image(
    ui: &mut egui::Ui,
    state: &mut UiState,
    v: &View,
    src: &str,
    alt: &str,
    column: f32,
) {
    const MAX_W: f32 = 420.0;
    const MAX_H: f32 = 300.0;
    let texture = if papo_core::preview::safe_remote_url(src) {
        if v.network {
            state.media.remote_image(src, src)
        } else {
            state.media.loaded_remote_image(src)
        }
        .and_then(|texture| texture.frame(ui.ctx()))
        .cloned()
    } else {
        None
    };
    let limit = Vec2::new(column.min(MAX_W), MAX_H);
    let size = match &texture {
        Some(texture) => {
            let natural = texture.size_vec2();
            let scale = (limit.x / natural.x.max(1.0))
                .min(limit.y / natural.y.max(1.0))
                .min(1.0);
            (natural * scale).max(Vec2::splat(1.0))
        }
        None => Vec2::new(limit.x.min(220.0), 90.0),
    };
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let corner = CornerRadius::same(crate::ui::theme::radius::CARD);
    match &texture {
        Some(texture) => super::widgets::photo(
            ui.painter(),
            rect,
            texture.id(),
            super::widgets::FULL_UV,
            corner,
            Color32::WHITE,
        ),
        None => {
            ui.painter().rect_filled(rect, corner, v.t.fill_soft);
            ui.painter().text(
                rect.center() - Vec2::new(0.0, 8.0),
                Align2::CENTER_CENTER,
                icon::IMAGE,
                text::icon(22.0),
                v.t.label_tertiary,
            );
            if !alt.is_empty() {
                ui.painter().text(
                    rect.center() + Vec2::new(0.0, 14.0),
                    Align2::CENTER_CENTER,
                    super::attachments::elide(alt, 28),
                    text::footnote(),
                    v.t.label_tertiary,
                );
            }
        }
    }
    ui.painter().rect_stroke(
        rect,
        corner,
        Stroke::new(1.0, v.t.separator),
        StrokeKind::Inside,
    );
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if response.clicked() {
        state.request_external_url(ui.ctx(), src.to_owned());
    }
}

/// Código em linha: monoespaçado a 0.9em, com borda e fundo tingido. Não
/// quebra no meio, a não ser que sozinho já passe da coluna.
fn code_chip(ui: &mut egui::Ui, v: &View, code: &str, scale: f32, column: f32) {
    let size = em() * scale;
    let pad = Vec2::new(0.32 * size, 0.08 * size);
    let galley = ui.painter().layout(
        code.to_owned(),
        mono(scale),
        v.t.label,
        (column - pad.x * 2.0).max(16.0),
    );
    let (rect, _) = ui.allocate_exact_size(galley.size() + pad * 2.0, Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(5),
        v.t.accent.gamma_multiply(0.14),
        Stroke::new(1.0, v.t.label.gamma_multiply(0.12)),
        StrokeKind::Inside,
    );
    ui.painter().galley(rect.min + pad, galley, v.t.label);
}

/// Menção: pastilha com o nome, que abre o cartão da pessoa, igual à foto e ao
/// nome do autor.
fn mention_pill(
    ui: &mut egui::Ui,
    state: &mut UiState,
    v: &View,
    user_id: &str,
    label: &str,
    scale: f32,
) {
    let size = em() * scale;
    let pad = Vec2::new(0.3 * size, 0.06 * size);
    let color = ui.visuals().hyperlink_color;
    let galley = ui
        .painter()
        .layout_no_wrap(format!("@{label}"), font(scale, true), color);
    let (rect, response) = ui.allocate_exact_size(galley.size() + pad * 2.0, Sense::click());
    let hovered = response.hovered();
    if hovered {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    ui.painter().rect_filled(
        rect,
        CornerRadius::same(5),
        v.t.accent.gamma_multiply(if hovered { 0.24 } else { 0.16 }),
    );
    ui.painter().galley(rect.min + pad, galley, color);
    if response.clicked() {
        let now = ui.input(|input| input.time);
        super::profile::open(state, user_id, super::profile::Anchor::Beside(rect), now);
    }
}

// ---------------------------------------------------------------------------
// Blocos
// ---------------------------------------------------------------------------

fn quote(ui: &mut egui::Ui, state: &mut UiState, v: &View, inner: &[Block]) {
    ui.add_space(0.15 * em());
    let row = ui.horizontal_top(|ui| {
        ui.add_space(2.0 + 0.7 * em());
        ui.vertical(|ui| draw_blocks(ui, state, v, inner, None));
    });
    let rect = row.response.rect;
    ui.painter().rect_filled(
        egui::Rect::from_min_size(rect.min, Vec2::new(2.0, rect.height())),
        CornerRadius::ZERO,
        line_color(v.t),
    );
    ui.add_space(0.15 * em());
}

fn list(ui: &mut egui::Ui, state: &mut UiState, v: &View, start: Option<u64>, items: &[ListItem]) {
    let size = em();
    let marker_font = font(1.0, false);
    let marker_of = |index: usize| match start {
        Some(first) => format!("{}.", first + index as u64),
        None => "•".to_owned(),
    };
    // O canal de marcadores tem 1.2em; números largos abrem mais um pouco.
    let widest = (0..items.len())
        .map(|index| {
            ui.painter()
                .layout_no_wrap(marker_of(index), marker_font.clone(), v.color)
                .size()
                .x
        })
        .fold(0.0, f32::max);
    let gutter = (1.2 * size).max(widest + 0.4 * size);
    let line_height = ui.fonts_mut(|fonts| fonts.row_height(&marker_font));

    ui.add_space(0.15 * em());
    for (index, item) in items.iter().enumerate() {
        ui.horizontal_top(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(gutter, line_height), Sense::hover());
            let anchor = egui::pos2(rect.max.x - 0.3 * size, rect.min.y);
            match item.task {
                Some(done) => {
                    let (glyph, color) = if done {
                        (icon::CHECK_SQUARE, v.t.accent)
                    } else {
                        (icon::SQUARE, v.t.label_secondary)
                    };
                    ui.painter()
                        .text(anchor, Align2::RIGHT_TOP, glyph, text::icon(size), color);
                }
                None => {
                    ui.painter().text(
                        anchor,
                        Align2::RIGHT_TOP,
                        marker_of(index),
                        marker_font.clone(),
                        v.color,
                    );
                }
            }
            ui.vertical(|ui| draw_blocks(ui, state, v, &item.blocks, None));
        });
        ui.add_space(0.05 * em());
    }
    ui.add_space(0.1 * em());
}

fn code_block(ui: &mut egui::Ui, v: &View, lang: Option<&str>, code: &str) {
    let (fill, border, color) = match v.t.appearance {
        Appearance::Dark => (
            Color32::from_rgb(0x09, 0x1b, 0x25),
            Color32::from_rgb(0x31, 0x50, 0x5f),
            Color32::from_rgb(0xe9, 0xf5, 0xfa),
        ),
        Appearance::Light => (
            Color32::from_rgb(0xdc, 0xe9, 0xef),
            Color32::from_rgb(0xb7, 0xcb, 0xd6),
            Color32::from_rgb(0x14, 0x28, 0x33),
        ),
    };
    let size = em();
    let margin = Margin::symmetric((0.9 * size) as i8, (0.8 * size) as i8);
    let frame_width = ui.available_width();
    let inner = (frame_width - f32::from(margin.left) - f32::from(margin.right) - 2.0).max(1.0);

    ui.add_space(0.5 * size);
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(margin)
        .show(ui, |ui| {
            ui.set_width(inner);
            if let Some(lang) = lang {
                ui.label(
                    RichText::new(lang)
                        .font(text::footnote())
                        .color(color.gamma_multiply(0.6)),
                );
            }
            ui.add(egui::Label::new(RichText::new(code).font(mono(1.0)).color(color)).wrap());
        });
    ui.add_space(0.5 * size);
}

/// Largura natural de uma célula, de uma linha só.
fn cell_width(ui: &egui::Ui, cell: &[Inline], bold: bool) -> f32 {
    let mut plain = String::new();
    let mut extra = 0.0;
    for inline in cell {
        match inline {
            Inline::Text { text, .. } => plain.push_str(text),
            Inline::Code(code) => {
                plain.push_str(code);
                extra += 0.64 * em();
            }
            Inline::Mention { label, .. } => {
                plain.push('@');
                plain.push_str(label);
                extra += 0.6 * em();
            }
            Inline::Image { alt, .. } => plain.push_str(alt),
            Inline::Break => plain.push(' '),
        }
    }
    let galley = ui
        .painter()
        .layout_no_wrap(plain, font(1.0, bold), Color32::WHITE);
    // Sobra de segurança: emoji e estilos medem um pouco mais que o texto.
    galley.size().x + extra + 4.0
}

fn table(
    ui: &mut egui::Ui,
    state: &mut UiState,
    v: &View,
    aligns: &[ColumnAlign],
    head: &[Vec<Inline>],
    rows: &[Vec<Vec<Inline>>],
) {
    let columns = rows.iter().map(Vec::len).fold(head.len(), usize::max);
    if columns == 0 {
        return;
    }
    let size = em();
    let (pad_x, pad_y) = (0.5 * size, 0.15 * size);
    let available = ui.available_width();
    let budget = (available - 2.0 * pad_x * columns as f32).max(columns as f32 * 8.0);

    // Cada coluna tem a largura do que há de mais largo nela; se a tabela não
    // cabe, todas encolhem juntas e o texto quebra dentro da célula.
    let mut widths = vec![0.0f32; columns];
    for (bold, cells) in std::iter::once((true, head)).chain(rows.iter().map(|r| (false, &r[..]))) {
        for (column, cell) in cells.iter().enumerate() {
            widths[column] = widths[column].max(cell_width(ui, cell, bold));
        }
    }
    let total: f32 = widths.iter().sum();
    if total > budget {
        let scale = budget / total;
        for width in &mut widths {
            *width *= scale;
        }
    }

    ui.add_space(0.15 * size);
    let left = ui.cursor().min.x;
    let mut y_range: Option<(f32, f32)> = None;
    let mut row_ends = Vec::new();
    let mut draw_row =
        |ui: &mut egui::Ui, state: &mut UiState, cells: &[Vec<Inline>], bold: bool| {
            let row = ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0 * pad_x;
                ui.add_space(pad_x);
                for (column, width) in widths.iter().enumerate() {
                    ui.vertical(|ui| {
                        ui.set_width(*width);
                        ui.add_space(pad_y);
                        if let Some(cell) = cells.get(column) {
                            // Uma linha só cabe no alinhamento da coluna; o
                            // que quebra em várias fica à esquerda.
                            let align = aligns.get(column).copied().unwrap_or_default();
                            let natural = cell_width(ui, cell, bold);
                            let offset = match align {
                                ColumnAlign::Left => 0.0,
                                ColumnAlign::Center => ((width - natural) / 2.0).max(0.0),
                                ColumnAlign::Right => (width - natural).max(0.0),
                            };
                            if offset > 1.0 {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 0.0;
                                    ui.add_space(offset);
                                    ui.vertical(|ui| {
                                        ui.set_width(width - offset);
                                        draw_inlines(ui, state, v, cell, 1.0, bold, None);
                                    });
                                });
                            } else {
                                draw_inlines(ui, state, v, cell, 1.0, bold, None);
                            }
                        }
                        ui.add_space(pad_y);
                    });
                }
            });
            let rect = row.response.rect;
            y_range = Some(match y_range {
                Some((top, _)) => (top, rect.max.y),
                None => (rect.min.y, rect.max.y),
            });
            row_ends.push(rect.max.y);
        };
    ui.scope(|ui| {
        draw_row(ui, state, head, true);
        for row in rows {
            draw_row(ui, state, row, false);
        }
    });

    let Some((top, bottom)) = y_range else {
        return;
    };
    let stroke = Stroke::new(1.0, line_color(v.t));
    let right = left + widths.iter().sum::<f32>() + 2.0 * pad_x * columns as f32;
    let painter = ui.painter();
    painter.rect_stroke(
        egui::Rect::from_min_max(egui::pos2(left, top), egui::pos2(right, bottom)),
        CornerRadius::ZERO,
        stroke,
        StrokeKind::Inside,
    );
    let mut x = left + pad_x;
    for width in widths.iter().take(columns - 1) {
        x += width + pad_x;
        painter.vline(x, top..=bottom, stroke);
        x += pad_x;
    }
    for y in row_ends.iter().take(row_ends.len().saturating_sub(1)) {
        painter.hline(left..=right, *y, stroke);
    }
    ui.add_space(0.15 * size);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::markdown::{parse, substitute_mentions};
    use crate::ui::theme;
    use egui::epaint::ClippedShape;
    use egui::{Rect, Shape, UiBuilder, pos2, vec2};
    use papo_core::state::MentionBinding;

    const LEFT: f32 = 24.0;
    const ID: &str = "11111111-2222-3333-4444-555555555555";
    /// `visual_bounding_rect` engorda os retângulos pela espessura do traço,
    /// mesmo com o traço por dentro.
    const STROKE: f32 = 1.0;

    struct Harness {
        ctx: egui::Context,
        state: UiState,
        store: Store,
        t: Tokens,
        blocks: Vec<Block>,
        width: f32,
        tags: Vec<String>,
    }

    impl Harness {
        fn new(
            appearance: Appearance,
            display: &str,
            bindings: &[MentionBinding],
            width: f32,
        ) -> Self {
            let ctx = egui::Context::default();
            theme::install_fonts(&ctx, None);
            let t = Tokens::new(appearance, None);
            theme::apply(&ctx, &t, false, 1.0);
            let blocks = parse(&substitute_mentions(display, bindings));
            let mut harness = Self {
                ctx,
                state: UiState::default(),
                store: Store::default(),
                t,
                blocks,
                width,
                tags: Vec::new(),
            };
            // Um quadro de aquecimento: as fontes só valem a partir do seguinte.
            harness.frame(vec![]);
            harness
        }

        fn text(appearance: Appearance, display: &str, width: f32) -> Self {
            Self::new(appearance, display, &[], width)
        }

        fn frame(&mut self, events: Vec<egui::Event>) -> (Vec<ClippedShape>, egui::PlatformOutput) {
            let input = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    pos2(0.0, 0.0),
                    vec2(self.width + 2.0 * LEFT, 4000.0),
                )),
                events,
                ..Default::default()
            };
            let Self {
                ctx,
                state,
                store,
                t,
                blocks,
                width,
                tags,
            } = self;
            let s = crate::i18n::Lang::En.strings();
            let view = View {
                t,
                s,
                store,
                color: t.label,
                edited: false,
                network: false,
                tags,
            };
            let mut output = ctx.run_ui(input, |ui| {
                ui.scope_builder(
                    UiBuilder::new()
                        .max_rect(Rect::from_min_size(pos2(LEFT, 20.0), vec2(*width, 3800.0))),
                    |ui| draw(ui, state, &view, blocks, *width),
                );
            });
            output.textures_delta.clear();
            (
                std::mem::take(&mut output.shapes),
                std::mem::take(&mut output.platform_output),
            )
        }

        /// Dois quadros: o segundo já tem os tamanhos do primeiro.
        fn settle(&mut self) -> Vec<ClippedShape> {
            self.frame(vec![]);
            self.frame(vec![]).0
        }
    }

    /// O que aparece de fato: cada forma recortada pelo seu próprio recorte.
    fn bounds(shapes: &[ClippedShape]) -> Rect {
        shapes
            .iter()
            .map(|clipped| {
                clipped
                    .shape
                    .visual_bounding_rect()
                    .intersect(clipped.clip_rect)
            })
            .filter(|rect| rect.is_finite())
            .fold(Rect::NOTHING, Rect::union)
    }

    fn text_rect(shapes: &[ClippedShape], needle: &str) -> Option<Rect> {
        shapes.iter().find_map(|clipped| match &clipped.shape {
            // O retângulo da galeria começa na margem da linha, antes do
            // recuo; o texto de verdade vai do primeiro ao último glifo.
            Shape::Text(text) if text.galley.text().contains(needle) => text
                .galley
                .rows
                .iter()
                .filter_map(|row| {
                    let first = row.glyphs.first()?;
                    let last = row.glyphs.last()?;
                    let rect = row.rect();
                    Some(Rect::from_min_max(
                        pos2(first.pos.x, rect.min.y),
                        pos2(last.pos.x + last.advance_width, rect.max.y),
                    ))
                })
                .map(|rect| rect.translate(text.pos.to_vec2()))
                .reduce(Rect::union),
            _ => None,
        })
    }

    const EVERYTHING: &str = "# Titulo grande\n## Dois\n### Tres\n#### Quatro\n\n\
        Texto com **negrito**, *itálico*, ~~riscado~~ e `código em linha` mais https://exemplo.com/um/caminho/bem/comprido/que/nao/quebra e [um link](https://exemplo.com) 👋\n\n\
        - item um com um texto bem longo que precisa quebrar linha dentro do item da lista\n  - aninhado com `codigo` e **negrito**\n    - ainda mais fundo e ainda mais fundo e ainda mais fundo\n- [x] feito\n- [ ] por fazer\n\n\
        3. três\n4. quatro\n10. dez\n\n\
        > citação com texto bem longo para quebrar linha dentro da citação\n> > citação dentro de citação\n\n\
        ---\n\n\
        | coluna um | dois | três com título longo |\n|--|--|--|\n| a | texto bem comprido dentro da célula que precisa quebrar | `código` |\n| b | c | d |\n\n\
        ```rust\nfn principal() { println!(\"linha de código muito longa que precisa quebrar dentro do painel\"); }\n```\n\n\
        `um_trecho_de_codigo_em_linha_sem_nenhum_espaco_que_e_maior_que_a_coluna_inteira_da_mensagem`\n\n\
        palavraenormesemespacosqueprecisaquebrarnomeiodapalavraporqueanaocabeemlinhaalguma\n\n\n\
        fim";

    #[test]
    fn nothing_paints_outside_the_column() {
        for appearance in [Appearance::Dark, Appearance::Light] {
            for width in [180.0, 260.0, 420.0, 640.0] {
                let mut harness = Harness::text(appearance, EVERYTHING, width);
                let shapes = harness.settle();
                let painted = bounds(&shapes);
                assert!(painted.is_positive(), "nada foi desenhado");
                assert!(
                    painted.min.x >= LEFT - STROKE,
                    "{appearance:?} {width}: passou da margem esquerda ({})",
                    painted.min.x
                );
                assert!(
                    painted.max.x <= LEFT + width + STROKE,
                    "{appearance:?} {width}: passou da margem direita ({} > {})",
                    painted.max.x,
                    LEFT + width
                );
            }
        }
    }

    #[test]
    fn blocks_are_stacked_without_overlap() {
        let mut harness = Harness::text(
            Appearance::Dark,
            "# um\n\ndois\n\n- tres\n\n> quatro\n\ncinco",
            320.0,
        );
        let shapes = harness.settle();
        let mut tops: Vec<f32> = ["um", "dois", "tres", "quatro", "cinco"]
            .iter()
            .map(|needle| {
                text_rect(&shapes, needle)
                    .unwrap_or_else(|| panic!("sem {needle}"))
                    .min
                    .y
            })
            .collect();
        let sorted = {
            let mut copy = tops.clone();
            copy.sort_by(f32::total_cmp);
            copy
        };
        assert_eq!(tops, sorted, "fora de ordem");
        tops.dedup_by(|a, b| (*a - *b).abs() < 1.0);
        assert_eq!(tops.len(), 5, "dois blocos na mesma altura");
    }

    #[test]
    fn list_markers_sit_on_the_first_line_of_their_item() {
        let mut harness = Harness::text(
            Appearance::Dark,
            "- primeiro item bem comprido que quebra em mais de uma linha para conferir\n1. numerado",
            200.0,
        );
        let shapes = harness.settle();
        let bullet = text_rect(&shapes, "•").expect("marcador");
        let first = text_rect(&shapes, "primeiro").expect("texto");
        assert!(bullet.min.x < first.min.x, "marcador à esquerda do texto");
        assert!(
            (bullet.center().y - first.center().y).abs() < first.height(),
            "marcador na mesma linha: {bullet:?} {first:?}"
        );
        let number = text_rect(&shapes, "1.").expect("número");
        let item = text_rect(&shapes, "numerado").expect("texto numerado");
        assert!(number.max.x <= item.min.x);
    }

    #[test]
    fn task_items_use_checkbox_glyphs_instead_of_bullets() {
        let mut harness = Harness::text(Appearance::Dark, "- [x] feito\n- [ ] falta", 300.0);
        let shapes = harness.settle();
        assert!(text_rect(&shapes, icon::CHECK_SQUARE).is_some());
        assert!(text_rect(&shapes, icon::SQUARE).is_some());
        assert!(text_rect(&shapes, "•").is_none());
    }

    #[test]
    fn table_cells_form_a_grid_with_borders() {
        let mut harness = Harness::text(
            Appearance::Dark,
            "| h1 | h2 |\n|--|--|\n| a | b |\n| c | d |",
            300.0,
        );
        let shapes = harness.settle();
        let cell =
            |needle: &str| text_rect(&shapes, needle).unwrap_or_else(|| panic!("sem {needle}"));
        let (a, b, c) = (cell("a"), cell("b"), cell("c"));
        assert!(b.min.x > a.max.x, "segunda coluna à direita");
        assert!((a.center().y - b.center().y).abs() < 1.0, "mesma linha");
        assert!(c.min.y > a.max.y - 1.0, "terceira linha abaixo");
        assert!(cell("h1").min.y < a.min.y);
        // Uma coluna é alinhada: a e c começam no mesmo x.
        assert!((a.min.x - c.min.x).abs() < 0.5);
        // Moldura + 1 divisória vertical + 2 divisórias horizontais.
        let lines = shapes
            .iter()
            .filter(|clipped| matches!(clipped.shape, Shape::LineSegment { .. }))
            .count();
        assert_eq!(lines, 3);
    }

    #[test]
    fn heading_is_bigger_than_body() {
        let mut harness = Harness::text(Appearance::Dark, "# grande\n\npequeno", 320.0);
        let shapes = harness.settle();
        let big = text_rect(&shapes, "grande").expect("título");
        let small = text_rect(&shapes, "pequeno").expect("corpo");
        // A altura da linha vem do mínimo do egui; o que cresce é a largura
        // de cada letra.
        assert!(big.width() / 6.0 > small.width() / 7.0 * 1.3);
    }

    #[test]
    fn plain_markdown_text_has_no_stray_markers() {
        let mut harness = Harness::text(Appearance::Dark, "**oi** _mundo_ ~~x~~", 320.0);
        let shapes = harness.settle();
        for clipped in &shapes {
            if let Shape::Text(text) = &clipped.shape {
                assert!(!text.galley.text().contains('*'));
                assert!(!text.galley.text().contains('~'));
            }
        }
    }

    fn pointer(at: egui::Pos2, pressed: Option<bool>) -> Vec<egui::Event> {
        let mut events = vec![egui::Event::PointerMoved(at)];
        if let Some(pressed) = pressed {
            events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        events
    }

    #[test]
    fn clicking_a_mention_opens_the_profile_card() {
        let binding = MentionBinding {
            start: 3,
            label: "Ana".into(),
            user_id: ID.into(),
        };
        let mut harness = Harness::new(Appearance::Dark, "oi @Ana, tudo bem?", &[binding], 320.0);
        let shapes = harness.settle();
        let pill = text_rect(&shapes, "@Ana").expect("pastilha").center();

        // Passar o mouse por cima mostra a mãozinha.
        let (_, platform) = harness.frame(pointer(pill, None));
        assert_eq!(platform.cursor_icon, egui::CursorIcon::PointingHand);
        assert!(harness.state.profile.is_none());

        harness.frame(pointer(pill, Some(true)));
        harness.frame(pointer(pill, Some(false)));
        let card = harness.state.profile.as_ref().expect("cartão aberto");
        assert_eq!(card.user_id, ID);
    }

    #[test]
    fn mention_name_with_markdown_characters_still_makes_a_pill() {
        let binding = MentionBinding {
            start: 0,
            label: "*_x_*".into(),
            user_id: ID.into(),
        };
        let mut harness = Harness::new(Appearance::Dark, "@*_x_*", &[binding], 320.0);
        let shapes = harness.settle();
        let pill = text_rect(&shapes, "@*_x_*").expect("pastilha com o nome inteiro");
        harness.frame(pointer(pill.center(), Some(true)));
        harness.frame(pointer(pill.center(), Some(false)));
        assert_eq!(
            harness.state.profile.as_ref().map(|c| c.user_id.as_str()),
            Some(ID)
        );
    }

    #[test]
    fn clicking_a_link_asks_before_opening() {
        let mut harness = Harness::text(
            Appearance::Dark,
            "veja [meu site](https://exemplo.com/x)",
            320.0,
        );
        let shapes = harness.settle();
        let word = text_rect(&shapes, "meu").expect("palavra do link").center();
        let (_, platform) = harness.frame(pointer(word, None));
        assert_eq!(platform.cursor_icon, egui::CursorIcon::PointingHand);
        harness.frame(pointer(word, Some(true)));
        harness.frame(pointer(word, Some(false)));
        let prompt = harness
            .state
            .external_link_prompt
            .as_ref()
            .expect("aviso de link");
        assert_eq!(prompt.url, "https://exemplo.com/x");
    }

    #[test]
    fn mailto_and_unsafe_links_do_nothing_on_click() {
        for source in ["[escreva](mailto:a@b.c)", "[escreva](javascript:alert(1))"] {
            let mut harness = Harness::text(Appearance::Dark, source, 320.0);
            let shapes = harness.settle();
            let word = text_rect(&shapes, "escreva").expect("palavra").center();
            harness.frame(pointer(word, Some(true)));
            harness.frame(pointer(word, Some(false)));
            assert!(harness.state.external_link_prompt.is_none(), "{source}");
            assert!(harness.state.profile.is_none());
        }
    }

    #[test]
    fn bare_urls_in_markdown_text_are_still_clickable() {
        let mut harness = Harness::text(Appearance::Dark, "**olha** https://exemplo.com/a", 320.0);
        let shapes = harness.settle();
        let word = text_rect(&shapes, "https://exemplo.com/a")
            .expect("url")
            .center();
        harness.frame(pointer(word, Some(true)));
        harness.frame(pointer(word, Some(false)));
        assert_eq!(
            harness
                .state
                .external_link_prompt
                .as_ref()
                .map(|p| p.url.as_str()),
            Some("https://exemplo.com/a")
        );
    }

    #[test]
    fn empty_and_html_only_messages_draw_nothing_and_do_not_panic() {
        for source in ["", "<script>alert(1)</script>", "<b></b>"] {
            let mut harness = Harness::text(Appearance::Light, source, 200.0);
            harness.settle();
        }
    }

    /// Há um retângulo preenchido com a cor da pastilha de menção.
    fn has_pill_fill(shapes: &[ClippedShape], t: &Tokens) -> bool {
        let pill = t.accent.gamma_multiply(0.16);
        shapes
            .iter()
            .any(|clipped| matches!(&clipped.shape, Shape::Rect(rect) if rect.fill == pill))
    }

    fn with_tags(source: &str, tags: &[&str]) -> (Vec<ClippedShape>, Tokens) {
        let mut harness = Harness::text(Appearance::Dark, source, 320.0);
        harness.tags = tags.iter().map(|tag| (*tag).to_owned()).collect();
        let shapes = harness.settle();
        (shapes, harness.t)
    }

    #[test]
    fn tags_get_a_pill_only_for_the_reader_or_everyone() {
        let (shapes, t) = with_tags("oi **@everyone**, venham", &[]);
        assert!(!has_pill_fill(&shapes, &t));

        let (shapes, t) = with_tags("oi **@everyone**, venham", &["everyone", "todos"]);
        assert!(has_pill_fill(&shapes, &t));
        let (shapes, t) = with_tags("oi @todos", &["everyone", "todos"]);
        assert!(has_pill_fill(&shapes, &t));

        // Cargo do leitor, inclusive com espaço no nome.
        let (shapes, t) = with_tags("chamando @Equipe de Design agora", &["equipe de design"]);
        assert!(has_pill_fill(&shapes, &t));
        let (shapes, t) = with_tags("chamando @Outro cargo", &["equipe de design"]);
        assert!(!has_pill_fill(&shapes, &t));
    }

    #[test]
    fn images_show_a_frame_with_the_alt_text_until_they_load() {
        let mut harness = Harness::text(
            Appearance::Dark,
            "![gato dormindo](https://exemplo.com/gato.png)",
            320.0,
        );
        let shapes = harness.settle();
        assert!(text_rect(&shapes, "gato dormindo").is_some());
    }

    #[test]
    fn column_alignment_moves_single_line_cells() {
        let place = |align: &str| {
            let source = format!("| cabeçalho bem largo |\n|{align}|\n| zz |");
            let mut harness = Harness::text(Appearance::Dark, &source, 320.0);
            let shapes = harness.settle();
            text_rect(&shapes, "zz").expect("célula").min.x
        };
        let left = place("---");
        let center = place(":-:");
        let right = place("--:");
        assert!(left < center && center < right, "{left} {center} {right}");
    }
}
