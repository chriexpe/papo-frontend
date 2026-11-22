//! The measures of message content (`docs/design.md` §9): chips in a line
//! of text, blocks, code, tables, emoji and the mention row. Each is a design
//! value; the content views take them from here.

use rune::TextStyle;

use super::{TYPE_BODY, TYPE_BODY_PT, TYPE_CAPTION, TYPE_SMALL, WEIGHT_SEMIBOLD};

/// Every chip in a line of text (inline code, a mention, a spoiler): its
/// corner radius, in points.
pub(crate) const CHIP_RADIUS_PT: f32 = 5.0;
/// Inline code's and a mention's padding on each side, in points.
pub(crate) const CHIP_PAD_PT: f32 = 4.0;
/// A spoiler's padding on each side, in points.
pub(crate) const SPOILER_PAD_PT: f32 = 2.0;
/// Inline code's outline, in points.
pub(crate) const CODE_CHIP_OUTLINE_PT: f32 = 1.0;
/// How strongly a link's underline draws at rest; hover and focus draw it
/// whole.
pub(crate) const LINK_UNDERLINE_ALPHA: f32 = 0.4;

/// Mono text at `type.small`, as inline code and code blocks set it.
pub(crate) const TYPE_CODE: TextStyle = TYPE_SMALL.mono();
/// A code block's lines, `type.small` mono at line height 1.5.
pub(crate) const TYPE_CODE_BLOCK: TextStyle = TYPE_CODE.lh(TYPE_SMALL.size * 1.5);
/// The pill under a link that shows its address, and the edited time.
pub(crate) const TYPE_HINT: TextStyle = TYPE_CAPTION.mono();

/// A heading of `level` (1 to 6): weight 600 at line height 1.25.
pub(crate) const fn type_heading(level: u8) -> TextStyle {
    let ratio = match level {
        1 => 1.5,
        2 => 1.25,
        3 => 1.08,
        4 => 1.0,
        _ => 0.93,
    };
    let size = TYPE_BODY_PT * ratio;
    TextStyle::new(size, WEIGHT_SEMIBOLD).lh(size * 1.25)
}

/// A code block: padding (vertical, horizontal), radius and outline, in
/// points.
pub(crate) const CODE_BLOCK_PAD_PT: (f32, f32) = (8.0, 10.0);
pub(crate) const CODE_BLOCK_RADIUS_PT: f32 = 8.0;
pub(crate) const CODE_BLOCK_OUTLINE_PT: f32 = 1.0;
/// From a code block's language label to its code, in points.
pub(crate) const CODE_LABEL_GAP_PT: f32 = 4.0;
/// A code block taller than this many lines scrolls inside.
pub(crate) const CODE_BLOCK_MAX_LINES: f32 = 20.0;
/// The code block's Copy button: height, horizontal padding, radius and
/// its inset from the block's top-right corner, in points.
pub(crate) const COPY_BUTTON_HEIGHT_PT: f32 = 26.0;
pub(crate) const COPY_BUTTON_PAD_PT: f32 = 8.0;
pub(crate) const COPY_BUTTON_RADIUS_PT: f32 = 7.0;
pub(crate) const COPY_BUTTON_INSET_PT: f32 = 6.0;
/// How long the Copy button reads ✓ "Copied", in seconds.
pub(crate) const COPIED_FOR_S: f64 = 1.5;

/// A list's marker column, in points (1.4 em of the text).
pub(crate) const LIST_MARKER_PT: f32 = TYPE_BODY_PT * 1.4;
/// Between a number and its item's text, in points (0.35 em).
pub(crate) const LIST_NUMBER_GAP_PT: f32 = TYPE_BODY_PT * 0.35;
/// Between a list's items, and between the blocks inside one, in points.
pub(crate) const LIST_ITEM_GAP_PT: f32 = 2.0;
/// A task's box: its side (1.05 em), outline and radius, in points.
pub(crate) const TASK_BOX_PT: f32 = TYPE_BODY_PT * 1.05;
pub(crate) const TASK_BOX_OUTLINE_PT: f32 = 1.5;
pub(crate) const TASK_BOX_RADIUS_PT: f32 = 4.0;
/// The ✓ inside a done task's box, in points square.
pub(crate) const TASK_CHECK_PT: f32 = TYPE_BODY_PT * 0.8;

/// A quote's bar and the indent of its text past the bar, in points.
pub(crate) const QUOTE_BAR_PT: f32 = 3.0;
pub(crate) const QUOTE_INDENT_PT: f32 = 10.0;
/// A thematic break's line, in points thick.
pub(crate) const RULE_PT: f32 = 1.0;

/// A table's text: 0.93 of the text size.
pub(crate) const TYPE_TABLE: TextStyle = {
    let size = TYPE_BODY_PT * 13.0 / 14.0;
    TextStyle::new(size, TYPE_BODY.weight).lh(size * 1.45)
};
/// A table cell's padding (vertical, horizontal) and lines, in points.
pub(crate) const TABLE_CELL_PAD_PT: (f32, f32) = (3.0, 10.0);
pub(crate) const TABLE_LINE_PT: f32 = 1.0;
/// A cell wraps past this many characters (CSS `28ch`).
pub(crate) const TABLE_CELL_CHARS: usize = 28;

/// A message of one to three emoji draws them at 3.4 times the text size,
/// on lines 1.1 times as tall.
pub(crate) const TYPE_JUMBO: TextStyle = {
    let size = TYPE_BODY_PT * 3.4;
    TextStyle::new(size, TYPE_BODY.weight).lh(size * 1.1)
};
/// The most emoji a jumbo message holds.
pub(crate) const JUMBO_MOST: usize = 3;

/// The corner radius of a highlighted message row (a mention, the message
/// being replied to or edited), in points.
pub(crate) const HIGHLIGHT_ROW_RADIUS_PT: f32 = 8.0;

/// The pill that shows a link's address or the edited time: padding
/// (vertical, horizontal), radius, and its distance from what it explains,
/// in points.
pub(crate) const HINT_PAD_PT: (f32, f32) = (3.0, 8.0);
pub(crate) const HINT_RADIUS_PT: f32 = 8.0;
pub(crate) const HINT_GAP_PT: f32 = 6.0;
/// How long the pointer rests on a link before its address shows, in
/// seconds.
pub(crate) const HINT_DELAY_S: f64 = 0.5;
/// The widest a hint grows before its words wrap, in points.
pub(crate) const HINT_MAX_WIDTH_PT: f32 = 480.0;
