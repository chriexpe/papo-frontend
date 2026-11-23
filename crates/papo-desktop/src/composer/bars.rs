//! The bar over the text pill while replying or editing (`docs/design.md`
//! §11): the pill's own header row, 32 pt on `surface.sidebar`, a hairline
//! under it. Replying: the reply icon, "Replying to {name} · {snippet}" on
//! one line, the @ switch (not when replying to yourself) and ✕. Editing:
//! the edit icon, "Editing message", "Esc to cancel" and ✕.
//!
//! Pure: what the original says is the shell's to work out
//! (`timeline::replies`); the presses go to the shell.

use papo_sdk::Uuid;
use rune::{El, Span, el, icon, row, text};

use super::mode::Mode;
use crate::design::{
    CHIP_ICON_PT, FOCUS_RING_PT, Look, MENTION_SWITCH_PT, PILL_OUTLINE_PT, RADIUS_COMPOSER,
    RADIUS_CONTROL, REPLY_BAR_HEIGHT_PT, REPLY_BAR_ICON_PT, SPACE_LG, SPACE_MD, SPACE_SM,
    TYPE_SMALL, WEIGHT_SEMIBOLD,
};
use crate::icons::{AT, EDIT, REPLY, X};
use crate::shell::Shell;
use crate::strings;
use crate::timeline::replies::OriginalText;

/// The element key of the bar.
pub(crate) const BAR_KEY: &str = "composer-bar";
/// The element key of the bar's ✕.
pub(crate) const BAR_CLOSE_KEY: &str = "composer-bar-close";
/// The element key of the @ switch.
pub(crate) const MENTION_SWITCH_KEY: &str = "reply-mention";

/// The bar for `mode`, if it has one. `original` is a reply's original in
/// words (none while it is looked up); `me` is the account's user.
pub(super) fn bar(
    mode: &Mode,
    original: Option<&OriginalText>,
    me: Uuid,
    look: &Look,
) -> Option<El> {
    let content = match mode {
        Mode::Writing { reply: None } => return None,
        Mode::Writing { reply: Some(reply) } => {
            let original = original.unwrap_or(&OriginalText::Earlier);
            let own = original.by(me);
            let line = reply_line(original, own, look);
            let switch = match original {
                OriginalText::Message { name, .. } if !own => {
                    Some(mention_switch(name, reply.mention, look))
                }
                _ => None,
            };
            row()
                .grow(1.0)
                .min_w(0.0)
                .center_y()
                .gap(look.space(SPACE_MD))
                .child(icon(&REPLY, REPLY_BAR_ICON_PT).color(look.palette.text_secondary))
                .child(line)
                .children(switch)
                .child(close(strings::reply_cancel(), Shell::reply_closed, look))
        }
        Mode::Editing(_) => {
            let lead = strings::edit_bar();
            let hint = strings::edit_esc();
            let line = format!("{lead}  {hint}");
            let spans = vec![
                Span::new(0, lead.len())
                    .weight(WEIGHT_SEMIBOLD)
                    .color(look.palette.text_primary),
            ];
            row()
                .grow(1.0)
                .min_w(0.0)
                .center_y()
                .gap(look.space(SPACE_MD))
                .child(icon(&EDIT, REPLY_BAR_ICON_PT).color(look.palette.text_secondary))
                .child(
                    text(line)
                        .font(look.font(TYPE_SMALL))
                        .color(look.palette.text_muted)
                        .spans(spans)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(close(strings::edit_cancel(), Shell::edit_closed, look))
        }
    };
    let inner_radius_pt = look.radius(RADIUS_COMPOSER) - PILL_OUTLINE_PT;
    Some(
        row()
            .key(BAR_KEY)
            .w_full()
            .h(REPLY_BAR_HEIGHT_PT)
            .pl(look.space(SPACE_LG))
            .pr(look.space(SPACE_SM))
            .center_y()
            .corners([inner_radius_pt, inner_radius_pt, 0.0, 0.0])
            .fill(look.palette.surface_sidebar)
            .border_sides([0.0, 0.0, PILL_OUTLINE_PT, 0.0], look.palette.line_subtle)
            .child(content),
    )
}

/// "Replying to Lia · is the build green?", or what stands for an
/// original that is missing.
fn reply_line(original: &OriginalText, own: bool, look: &Look) -> El {
    let (line, spans) = match original {
        OriginalText::Message { name, snippet, .. } => {
            let lead = if own {
                strings::reply_bar_self().to_owned()
            } else {
                strings::reply_bar(name)
            };
            let mut spans = Vec::new();
            if !own && let Some(at) = lead.find(name.as_str()) {
                spans.push(
                    Span::new(at, at + name.len())
                        .weight(WEIGHT_SEMIBOLD)
                        .color(look.palette.text_primary),
                );
            }
            let line = if snippet.is_empty() {
                lead
            } else {
                format!("{lead} · {snippet}")
            };
            (line, spans)
        }
        OriginalText::Deleted => {
            let line = strings::reply_deleted().to_owned();
            let spans = vec![Span::new(0, line.len()).italic()];
            (line, spans)
        }
        OriginalText::Earlier => {
            let line = strings::reply_earlier().to_owned();
            let spans = vec![Span::new(0, line.len()).italic()];
            (line, spans)
        }
    };
    text(line)
        .font(look.font(TYPE_SMALL))
        .color(look.palette.text_muted)
        .spans(spans)
        .truncate()
        .grow(1.0)
        .min_w(0.0)
}

/// The @ switch: on, the link colour on `accent.soft`; off, `text.muted`
/// inside a `line.subtle` outline. Spoken "Mention Lia, on".
fn mention_switch(name: &str, on: bool, look: &Look) -> El {
    let (height_pt, inset_pt) = MENTION_SWITCH_PT;
    let switch = row()
        .key(MENTION_SWITCH_KEY)
        .h(height_pt)
        .px(inset_pt)
        .center()
        .radius(height_pt / 2.0)
        .role("switch")
        // Spoken "Mention Lia", on or off as the system says it.
        .label(strings::reply_mention(name))
        .checked(on)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(Shell::reply_mention_toggled)
        .child(icon(&AT, CHIP_ICON_PT).color(if on {
            look.palette.link
        } else {
            look.palette.text_muted
        }));
    if on {
        switch.fill(look.palette.accent_soft)
    } else {
        switch.border(PILL_OUTLINE_PT, look.palette.line_subtle)
    }
}

/// ✕, named `label`.
fn close(label: &'static str, pressed: fn(&mut Shell), look: &Look) -> El {
    el().key(BAR_CLOSE_KEY)
        .size(
            REPLY_BAR_HEIGHT_PT - look.space(SPACE_SM) * 2.0,
            REPLY_BAR_HEIGHT_PT - look.space(SPACE_SM) * 2.0,
        )
        .center()
        .radius(look.radius(RADIUS_CONTROL))
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(pressed)
        .child(icon(&X, REPLY_BAR_ICON_PT).color(look.palette.text_secondary))
}
