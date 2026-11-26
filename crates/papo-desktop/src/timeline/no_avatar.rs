//! A message row's content without avatars (`docs/design.md` §6, §8): at
//! Extra compact, or at any spacing with Show avatars off. The name sits
//! above the text (Stack messages) or beside it, a continued row keeping
//! the name's place, its cards and media on the content column below; with
//! no time column the send mark goes after the name, or at the right end of
//! a continued row's first line.
//!
//! Owns that content's layout; the lead column and the row are
//! `message_row`'s.

use rune::{El, col, row};

use super::message_row::name_text;
use super::row_media::Body;
use super::row_parts::Parts;
use super::view::Style;
use crate::design::{MESSAGE_BLOCK_GAP, MESSAGE_HEADER_GAP, MESSAGE_NAME_GAP};

/// A row's content without avatars: the name above the text (stacked) or
/// beside it (inline), a continued row keeping the name's place. `mark` is
/// the send or change mark when no time column holds it: after the name,
/// or at the right end of a continued row's first line.
pub(super) fn without_avatars(
    parts: &Parts<'_>,
    body: Body,
    mark: Option<El>,
    style: &Style,
    name: impl Fn() -> El,
) -> El {
    let look = style.look;
    let named = |mark: Option<El>| {
        row()
            .gap(look.space(MESSAGE_HEADER_GAP))
            .items_end()
            .child(name())
            .children(mark)
    };
    let (mark_by_name, mark_at_end) = if parts.continued {
        (None, mark)
    } else {
        (mark, None)
    };
    let content = if style.layout.stacked {
        let mut stack = col().gap(look.space(MESSAGE_BLOCK_GAP));
        if !parts.continued {
            stack = stack.child(named(mark_by_name));
        }
        stack.child(body.joined(&look))
    } else {
        // A continued row keeps the hidden name's width, so its text stays
        // on the column.
        let hidden = || {
            // Only a placeholder: it opens no card.
            name_text(&parts.name, &look).opacity(0.0).a11y_hidden()
        };
        let name = if parts.continued {
            hidden()
        } else {
            named(mark_by_name)
        };
        let beside = |name: El, content: El| {
            row()
                .gap(look.space(MESSAGE_NAME_GAP))
                .items_start()
                .child(name)
                .child(col().grow(1.0).min_w(0.0).child(content))
        };
        if body.blocks.is_empty() {
            beside(name, body.joined(&look))
        } else {
            // Cards and media start on the content column, where the name does, whatever the
            // name's width; the text and what follows the blocks stay beside the name.
            let after = (!body.after.is_empty()).then(|| {
                let after = col()
                    .gap(look.space(MESSAGE_BLOCK_GAP))
                    .children(body.after);
                beside(hidden(), after)
            });
            col()
                .w_full()
                .gap(look.space(MESSAGE_BLOCK_GAP))
                .child(beside(name, body.text))
                .children(body.blocks)
                .children(after)
        }
    };
    match mark_at_end {
        Some(mark) => row()
            .w_full()
            .gap(look.space(MESSAGE_HEADER_GAP))
            .items_start()
            .child(content.grow(1.0).min_w(0.0))
            .child(mark),
        None => content,
    }
}
