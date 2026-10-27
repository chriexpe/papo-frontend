//! Inline sticker presentation. Drafts and network payloads keep shortcodes;
//! the desktop editor sees one character per sticker, including undo/selection.

use std::{cell::RefCell, collections::HashMap};

use egui::{Color32, FontId, Rect, Vec2};

use crate::state::{CustomEmoji, Emoji, Store};

#[derive(Default)]
pub struct StickerText {
    entries: Vec<(String, String)>,
    symbols: HashMap<(String, String), char>,
    preview: RefCell<PreviewCache>,
}

#[derive(Default)]
struct PreviewCache {
    source: String,
    styles: Vec<papo_core::markdown::composer::StyledRange>,
}

impl StickerText {
    fn entry(&self, symbol: char) -> Option<&(String, String)> {
        let index = (symbol as u32).checked_sub(0xf0000)? as usize;
        self.entries.get(index)
    }

    pub fn encode(&mut self, text: &str, emojis: &[CustomEmoji]) -> String {
        let mut names = HashMap::new();
        for emoji in emojis {
            let key = (emoji.id.clone(), emoji.name.clone());
            let symbol = if let Some(symbol) = self.symbols.get(&key) {
                *symbol
            } else {
                let Some(symbol) = char::from_u32(0xf0000 + self.entries.len() as u32)
                    .filter(|symbol| (*symbol as u32) <= 0xffffd)
                else {
                    continue;
                };
                self.entries.push(key.clone());
                self.symbols.insert(key, symbol);
                symbol
            };
            names.entry(emoji.name.as_str()).or_insert(symbol);
        }
        let mut out = String::new();
        let mut rest = text;
        while !rest.is_empty() {
            if let Some(after) = rest.strip_prefix(':')
                && let Some(end) = after.find(':')
                && let Some(symbol) = names.get(&after[..end])
            {
                out.push(*symbol);
                rest = &after[end + 1..];
            } else {
                let ch = rest.chars().next().unwrap();
                out.push(ch);
                rest = &rest[ch.len_utf8()..];
            }
        }
        out
    }

    pub fn decode(&self, shown: &str) -> String {
        let mut out = String::new();
        for ch in shown.chars() {
            if let Some((_, name)) = self.entry(ch) {
                out.push(':');
                out.push_str(name);
                out.push(':');
            } else {
                out.push(ch);
            }
        }
        out
    }

    pub fn raw_caret(&self, shown: &str, caret: usize) -> usize {
        shown
            .chars()
            .take(caret)
            .map(|ch| {
                self.entry(ch)
                    .map_or(1, |(_, name)| name.chars().count() + 2)
            })
            .sum()
    }

    pub fn shown_caret(&self, shown: &str, caret: usize) -> usize {
        let mut raw = 0;
        for (index, ch) in shown.chars().enumerate() {
            if raw >= caret {
                return index;
            }
            raw += self
                .entry(ch)
                .map_or(1, |(_, name)| name.chars().count() + 2);
        }
        shown.chars().count()
    }

    pub fn layout(
        &self,
        ui: &egui::Ui,
        shown: &str,
        font: FontId,
        color: Color32,
        width: f32,
    ) -> std::sync::Arc<egui::Galley> {
        self.layout_inner(ui, shown, font, color, width, None)
    }

    /// The display buffer is unchanged: editable syntax, sticker slots, copy,
    /// selection and IME all keep the same character positions.
    pub fn markdown_layout(
        &self,
        ui: &egui::Ui,
        shown: &str,
        font: FontId,
        t: &super::theme::Tokens,
        width: f32,
    ) -> std::sync::Arc<egui::Galley> {
        self.layout_inner(ui, shown, font, t.label, width, Some(t))
    }

    fn layout_inner(
        &self,
        ui: &egui::Ui,
        shown: &str,
        font: FontId,
        color: Color32,
        width: f32,
        preview: Option<&super::theme::Tokens>,
    ) -> std::sync::Arc<egui::Galley> {
        let mut job = if let Some(t) = preview {
            let mut cache = self.preview.borrow_mut();
            if cache.source != shown {
                cache.styles = papo_core::markdown::composer::styles(shown);
                cache.source = shown.to_owned();
            }
            super::composer_format::layout_job(shown, &cache.styles, &font, color, t)
        } else {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                shown,
                0.0,
                egui::TextFormat {
                    font_id: font.clone(),
                    color,
                    ..Default::default()
                },
            );
            job
        };
        let source = std::mem::take(&mut job.text);
        let sections = std::mem::take(&mut job.sections);
        for section in sections {
            if section.byte_range.is_empty() {
                job.append("", 0.0, section.format.clone());
            }
            for ch in source[section.byte_range.start.0..section.byte_range.end.0].chars() {
                let format = if self.entry(ch).is_some() {
                    // Reserve the real sticker advance independently of text
                    // styling, including stickers inside a heading/emphasis.
                    let size = section.format.font_id.size.max(18.0);
                    let mut slot_font = FontId::proportional(size + 2.0);
                    let advance = ui.fonts_mut(|fonts| fonts.glyph_width(&slot_font, ch));
                    if advance > 0.0 {
                        slot_font.size *= (size + 2.0) / advance;
                    }
                    egui::TextFormat {
                        font_id: slot_font,
                        color: Color32::TRANSPARENT,
                        line_height: Some(size + 2.0),
                        ..Default::default()
                    }
                } else {
                    section.format.clone()
                };
                job.append(&ch.to_string(), 0.0, format);
            }
        }
        job.wrap.max_width = width;
        job.keep_trailing_whitespace = true;
        ui.fonts_mut(|fonts| fonts.layout_job(job))
    }

    pub fn paint(
        &self,
        ui: &mut egui::Ui,
        output: &egui::text_edit::TextEditOutput,
        t: &super::theme::Tokens,
        media: &mut crate::media::MediaStore,
        store: &Store,
    ) {
        let mut clipped = ui.new_child(egui::UiBuilder::new());
        clipped.set_clip_rect(output.text_clip_rect.intersect(ui.clip_rect()));
        for row in &output.galley.rows {
            for glyph in &row.glyphs {
                let Some((id, _)) = self.entry(glyph.chr) else {
                    continue;
                };
                let slot = Rect::from_min_size(
                    output.galley_pos + row.pos.to_vec2() + Vec2::new(glyph.pos.x, 0.0),
                    Vec2::new(glyph.advance_width, row.size.y),
                );
                let size = (slot.width().min(slot.height()) - 2.0).max(0.0);
                let rect = Rect::from_center_size(slot.center(), Vec2::splat(size));
                super::emoji::draw_reaction(
                    &mut clipped,
                    t,
                    media,
                    store,
                    &Emoji::Custom(id.clone()),
                    rect,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_composer_uses_the_same_line_metrics_as_its_text() {
        let ctx = egui::Context::default();
        super::super::theme::install_fonts(&ctx, None);
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let editor = StickerText::default();
            for size in [11.0, 13.0, 18.0, 24.0] {
                let font = FontId::proportional(size);
                let empty = editor.layout(ui, "", font.clone(), Color32::WHITE, 300.0);
                let typed = editor.layout(ui, "Hello", font, Color32::WHITE, 300.0);
                assert!((empty.size().y - typed.size().y).abs() < 0.01);
                let caret = egui::text::CCursor::new(0);
                assert_eq!(
                    empty.pos_from_cursor(caret).y_range(),
                    typed.pos_from_cursor(caret).y_range()
                );
            }
        });
        output.textures_delta.clear();
    }

    #[test]
    fn inline_stickers_reserve_a_full_square_in_the_text_editor() {
        for scale in [1.0, 1.25, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(scale);
            super::super::theme::install_fonts(&ctx, None);
            let mut editor = StickerText::default();
            let emojis = [CustomEmoji {
                id: "test".into(),
                name: "test".into(),
                blob: None,
            }];
            let mut store = Store::default();
            store.emojis = emojis.to_vec();
            let tokens =
                super::super::theme::Tokens::new(super::super::theme::Appearance::Dark, None);
            let mut media = crate::media::MediaStore::new(None);
            let mut output = ctx.run_ui(Default::default(), |ui| {
                for size in [11.0_f32, 13.0, 18.0, 24.0] {
                    for width in [60.0, 300.0] {
                        for text in [":test:", "text :test: ", ":test::test: end"] {
                            let mut shown = editor.encode(text, &emojis);
                            let font = FontId::proportional(size);
                            let mut layouter = |ui: &egui::Ui, text: &dyn egui::TextBuffer, width| {
                                editor.layout(ui, text.as_str(), font.clone(), Color32::WHITE, width)
                            };
                            let field = egui::TextEdit::multiline(&mut shown)
                                .font(font.clone())
                                .layouter(&mut layouter)
                                .desired_width(width)
                                .show(ui);
                            for row in &field.galley.rows {
                                for glyph in &row.glyphs {
                                    if editor.entry(glyph.chr).is_some() {
                                        assert!(
                                            glyph.advance_width >= size.max(18.0),
                                            "sticker squeezed to {} points: {text:?}, font={size}, scale={scale}",
                                            glyph.advance_width,
                                        );
                                    }
                                }
                            }
                            editor.paint(ui, &field, &tokens, &mut media, &store);
                        }
                    }
                }
            });
            let mut painted = 0;
            for shape in &output.shapes {
                if let egui::Shape::Rect(rect) = &shape.shape
                    && rect.stroke.color == tokens.separator
                {
                    painted += 1;
                    assert!(
                        rect.rect.width() >= 17.5,
                        "sticker too small: {:?}",
                        rect.rect
                    );
                    assert!((rect.rect.width() - rect.rect.height()).abs() < 0.01);
                    assert!(
                        shape.clip_rect.expand(0.01).contains_rect(rect.rect),
                        "sticker clipped: {:?} outside {:?}",
                        rect.rect,
                        shape.clip_rect,
                    );
                }
            }
            assert_eq!(painted, 32);
            output.textures_delta.clear();
        }
    }

    #[test]
    fn stickers_round_trip_with_unicode_selection_and_undo() {
        let emoji = CustomEmoji {
            id: "test".into(),
            name: "test".into(),
            blob: None,
        };
        let mut editor = StickerText::default();
        let shown = editor.encode("é :test: hi :missing:", &[emoji]);
        assert_eq!(editor.decode(&shown), "é :test: hi :missing:");
        assert_eq!(editor.raw_caret(&shown, 3), 8);
        assert_eq!(editor.shown_caret(&shown, 8), 3);
        let mut deleted = shown.clone();
        let at = deleted.char_indices().nth(2).unwrap().0;
        deleted.remove(at);
        assert_eq!(editor.decode(&deleted), "é  hi :missing:");
        editor.encode("", &[]);
        assert_eq!(editor.decode(&shown), "é :test: hi :missing:");
    }
}
