//! The measures of message content (`docs/design.md` §9): chips in a line
//! of text, blocks, code, tables, emoji and the mention row. Each is a design
//! value; the content views take them from here.

use rune::TextStyle;

use super::{
    Font, Radius, Space, TYPE_BODY, TYPE_BODY_PT, TYPE_CAPTION, TYPE_SMALL, TextMultiple,
    WEIGHT_SEMIBOLD,
};

/// Every chip in a line of text (inline code, a mention, a spoiler): its
/// corner radius.
pub(crate) const CHIP_RADIUS: Radius = Radius(5.0);
/// Inline code's and a mention's padding on each side.
pub(crate) const CHIP_PAD: Space = Space(4.0);
/// A spoiler's padding on each side, in points.
pub(crate) const SPOILER_PAD_PT: f32 = 2.0;
/// Inline code's outline, in points.
pub(crate) const CODE_CHIP_OUTLINE_PT: f32 = 1.0;
/// How strongly a link's underline draws at rest; hover and focus draw it
/// whole.
pub(crate) const LINK_UNDERLINE_ALPHA: f32 = 0.4;

/// Mono text at `type.small`, as inline code and code blocks set it.
pub(crate) const TYPE_CODE: Font = TYPE_SMALL.mono();
/// A code block's lines, `type.small` mono at line height 1.5.
pub(crate) const TYPE_CODE_BLOCK: Font = Font(TYPE_CODE.0.lh(TYPE_SMALL.0.size * 1.5));
/// The pill under a link that shows its address, and the edited time.
pub(crate) const TYPE_HINT: Font = TYPE_CAPTION.mono();

/// A heading of `level` (1 to 6): weight 600 at line height 1.25.
pub(crate) const fn type_heading(level: u8) -> Font {
    let ratio = match level {
        1 => 1.5,
        2 => 1.25,
        3 => 1.08,
        4 => 1.0,
        _ => 0.93,
    };
    let size = TYPE_BODY_PT * ratio;
    Font(TextStyle::new(size, WEIGHT_SEMIBOLD).lh(size * 1.25))
}

/// A code block: padding (vertical, horizontal) and outline, in points, and
/// its corner radius.
pub(crate) const CODE_BLOCK_PAD_PT: (f32, f32) = (8.0, 10.0);
pub(crate) const CODE_BLOCK_RADIUS: Radius = Radius(8.0);
pub(crate) const CODE_BLOCK_OUTLINE_PT: f32 = 1.0;
/// From a code block's language label to its code.
pub(crate) const CODE_LABEL_GAP: Space = Space(4.0);
/// A code block taller than this many lines scrolls inside.
pub(crate) const CODE_BLOCK_MAX_LINES: f32 = 20.0;
/// The code block's Copy button: height, horizontal padding and its inset
/// from the block's top-right corner, in points.
pub(crate) const COPY_BUTTON_HEIGHT_PT: f32 = 26.0;
pub(crate) const COPY_BUTTON_PAD_PT: f32 = 8.0;
pub(crate) const COPY_BUTTON_INSET_PT: f32 = 6.0;
/// How long the Copy button reads ✓ "Copied", in seconds.
pub(crate) const COPIED_FOR_S: f64 = 1.5;

/// A list's marker column: 1.4 em of the text.
pub(crate) const LIST_MARKER: TextMultiple = TextMultiple(1.4);
/// Between a number and its item's text: 0.35 em.
pub(crate) const LIST_NUMBER_GAP: TextMultiple = TextMultiple(0.35);
/// Between a list's items, and between the blocks inside one, in points.
pub(crate) const LIST_ITEM_GAP_PT: f32 = 2.0;
/// A task's box: its side (1.05 em), outline in points, and radius.
pub(crate) const TASK_BOX: TextMultiple = TextMultiple(1.05);
pub(crate) const TASK_BOX_OUTLINE_PT: f32 = 1.5;
pub(crate) const TASK_BOX_RADIUS: Radius = Radius(4.0);
/// The ✓ inside a done task's box, square: 0.8 em.
pub(crate) const TASK_CHECK: TextMultiple = TextMultiple(0.8);

/// A quote's bar, in points, and the indent of its text past the bar.
pub(crate) const QUOTE_BAR_PT: f32 = 3.0;
pub(crate) const QUOTE_INDENT: Space = Space(10.0);
/// A thematic break's line, in points thick.
pub(crate) const RULE_PT: f32 = 1.0;

/// A table's text: 0.93 of the text size.
pub(crate) const TYPE_TABLE: Font = {
    let size = TYPE_BODY_PT * 13.0 / 14.0;
    Font(TextStyle::new(size, TYPE_BODY.0.weight).lh(size * 1.45))
};
/// A table cell's padding (vertical, horizontal) and lines, in points.
pub(crate) const TABLE_CELL_PAD_PT: (f32, f32) = (3.0, 10.0);
pub(crate) const TABLE_LINE_PT: f32 = 1.0;
/// A cell wraps past this many characters (CSS `28ch`).
pub(crate) const TABLE_CELL_CHARS: usize = 28;

/// A message of one to three emoji draws them at 3.4 times the text size,
/// on lines 1.1 times as tall.
pub(crate) const TYPE_JUMBO: Font = {
    let size = TYPE_BODY_PT * 3.4;
    Font(TextStyle::new(size, TYPE_BODY.0.weight).lh(size * 1.1))
};
/// The most emoji a jumbo message holds.
pub(crate) const JUMBO_MOST: usize = 3;

/// A loading custom emoji's square: its corner radius (§9).
pub(crate) const EMOJI_LOADING_RADIUS: Radius = Radius(4.0);

/// Inter's ascender and descender in units of the text size (its `hhea`: 1984 and 494 of
/// 2048), which with the line's half-leading place the baseline in a line box.
const INTER_ASCENT: f32 = 1984.0 / 2048.0;
const INTER_DESCENT: f32 = 494.0 / 2048.0;

/// How far a custom emoji stays inside its line box at the top and at the bottom, in points:
/// Rune rounds an inline image's box to device pixels, which could otherwise push past the
/// line's edge by half a pixel and grow it (§9: a line does not grow).
const EMOJI_INSET_PT: f32 = 0.5;

/// A custom emoji's box in a line of `font` (§9): its height and how far it reaches below the
/// baseline, in points. Inline it fills the line box, so a line does not grow; in a jumbo
/// line (`jumbo`) it is the text size, as a Unicode emoji's glyph is.
pub(crate) fn emoji_box(font: TextStyle, jumbo: bool) -> (f32, f32) {
    let below_baseline = (font.line_height - font.size * (INTER_ASCENT + INTER_DESCENT)) / 2.0
        + font.size * INTER_DESCENT;
    if jumbo {
        // Centred on the line, as the glyphs it stands among are.
        let height = font.size;
        let below = below_baseline - (font.line_height - height) / 2.0;
        return (height, below.max(0.0));
    }
    (
        font.line_height - 2.0 * EMOJI_INSET_PT,
        below_baseline - EMOJI_INSET_PT,
    )
}

/// The corner radius of a highlighted message row (a mention, the message
/// being replied to or edited).
pub(crate) const HIGHLIGHT_ROW_RADIUS: Radius = Radius(8.0);

/// The pill that shows a link's address or the edited time: padding
/// (vertical, horizontal), radius, and its distance from what it explains,
/// in points.
pub(crate) const HINT_PAD_PT: (f32, f32) = (3.0, 8.0);
pub(crate) const HINT_RADIUS: Radius = Radius(8.0);
pub(crate) const HINT_GAP_PT: f32 = 6.0;
/// How long the pointer rests on a link before its address shows, in
/// seconds.
pub(crate) const HINT_DELAY_S: f64 = 0.5;
/// The widest a hint grows before its words wrap, in points.
pub(crate) const HINT_MAX_WIDTH_PT: f32 = 480.0;
