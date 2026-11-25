//! The reading controls floating over the timeline's bottom right
//! (`docs/design.md` §6, Reading controls): ⌛ over ↓, ↓'s arrivals badge,
//! and the pill at the timeline's foot while ⌛ pages toward the skipped
//! block, with Cancel.
//!
//! Pure: what shows and what a press means are `reading`'s; the presses go
//! to the host.

use rune::{El, LoopSpec, col, icon, row, text};

use super::reading::Jump;
use super::view::{Style, TimelineHost};
use super::{OpenTimeline, TimelineKey};
use crate::badges::{Count, count_pill};
use crate::design::{
    FOCUS_RING_PT, HEADER_PILL_HEIGHT, JUMP_BUTTON_PT, JUMP_GAP, JUMP_ICON_PT, JUMP_INSET, Look,
    MENTION_PILL_OVERHANG_PT, MENTION_PILL_RING_PT, RADIUS_CONTROL, RADIUS_PILL, SPACE_LG,
    SPACE_SM, SPINNER_PERIOD_S, SPINNER_PT, Surfaces, TYPE_SMALL, TYPE_SMALL_MEDIUM,
};
use crate::icons::{ARROW_DOWN, HOURGLASS, SPINNER};
use crate::strings;

/// ⌛ over ↓ at the bottom right, each when it shows; none when neither
/// does.
pub(super) fn jump_stack<S: TimelineHost>(open: &OpenTimeline, style: &Style) -> Option<El> {
    let place = open.place(style.hourglass);
    let reading = &open.reading;
    let has_messages = open
        .items
        .iter()
        .any(|item| matches!(item.as_ref(), papo_sdk_ui::TimelineItem::Message { .. }));
    let hourglass = reading.hourglass_shown(style.hourglass) && open.seek.is_none();
    let down = reading.down_shown(place) && has_messages;
    if !hourglass && !down {
        return None;
    }
    let key = open.key;
    let look = &style.look;
    let mut stack = col()
        .key("jump")
        .pin(
            None,
            None,
            Some(look.space(JUMP_INSET)),
            // Above the composer, which floats over the rows.
            Some(style.insets_pt[1] + look.space(JUMP_INSET)),
        )
        .gap(look.space(JUMP_GAP))
        .items_end();
    if hourglass {
        stack = stack.child(
            round_button("hourglass", &HOURGLASS, strings::hourglass(), look)
                .on_press(move |host: &mut S| host.hourglass_pressed(key)),
        );
    }
    if down {
        let action = match reading.next_jump(place) {
            Jump::ClearBlock => strings::clear_block(),
            Jump::Message(_) => strings::first_new(),
            Jump::Latest => strings::latest(),
        };
        let arrivals = reading.arrivals();
        let label = if arrivals > 0 {
            strings::with_arrivals(action, &strings::arrivals(arrivals))
        } else {
            action.to_owned()
        };
        let mut button = round_button("down", &ARROW_DOWN, &label, look)
            .on_press_tx(move |host: &mut S, tx| host.down_pressed(key, tx));
        if arrivals > 0 {
            let badge = count_pill(
                Count::exact(arrivals),
                (look.palette.accent, look.palette.accent_on),
                Some(look.palette.surface_content),
                look,
            );
            let overhang = MENTION_PILL_OVERHANG_PT + MENTION_PILL_RING_PT;
            button = button.child(
                row()
                    .key("arrivals")
                    .pin(None, Some(-overhang), Some(-overhang), None)
                    .child(badge),
            );
        }
        stack = stack.child(button);
    }
    Some(stack)
}

/// A round glass button holding `glyph`, named `label`.
fn round_button(key: &'static str, glyph: &'static rune::Icon, label: &str, look: &Look) -> El {
    row()
        .key(key)
        .size(JUMP_BUTTON_PT, JUMP_BUTTON_PT)
        .center()
        .radius(JUMP_BUTTON_PT / 2.0)
        .pill_surface(look)
        .role("button")
        .label(label.to_owned())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .child(
            icon(glyph, JUMP_ICON_PT)
                .color(look.palette.text_secondary)
                .a11y_hidden(),
        )
}

/// "Loading earlier unread messages…" with Cancel, at the timeline's foot,
/// while ⌛ pages toward the block.
pub(super) fn seeking_pill<S: TimelineHost>(key: TimelineKey, look: &Look) -> El {
    let cancel = text(strings::seek_cancel())
        .key("seek-cancel")
        .font(look.font(TYPE_SMALL_MEDIUM))
        .color(look.palette.link)
        .role("button")
        .focusable(true)
        .radius(look.radius(RADIUS_CONTROL))
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(move |host: &mut S| host.seek_cancelled(key));
    let pill = row()
        .key("seeking")
        .h(look.space(HEADER_PILL_HEIGHT))
        .px(look.space(SPACE_LG))
        .gap(look.space(SPACE_LG))
        .center_y()
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .child(
            icon(&SPINNER, SPINNER_PT)
                .color(look.palette.text_secondary)
                .looping(LoopSpec::rotation(SPINNER_PERIOD_S))
                .a11y_hidden(),
        )
        .child(
            text(strings::seeking())
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_primary),
        )
        .child(cancel);
    row()
        .pin(Some(0.0), None, Some(0.0), Some(look.space(JUMP_INSET)))
        .justify_center()
        .gap(look.space(SPACE_SM))
        .child(pill)
}

/// The pill a save ends with (§14.7), in the seek pill's shape at the foot of the timeline (or
/// of the viewer, while it is open): "Saved praia.jpg" and Show in folder, clear of the
/// composer floating `below_pt` over the rows.
pub(crate) fn saved_pill<S: TimelineHost>(name: &str, below_pt: f32, look: &Look) -> El {
    let show = text(strings::saved_show())
        .key("saved-show")
        .font(look.font(TYPE_SMALL_MEDIUM))
        .color(look.palette.link)
        .role("button")
        .focusable(true)
        .radius(look.radius(RADIUS_CONTROL))
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(|host: &mut S, tx| host.saved_pill_shown(tx));
    let pill = row()
        .key("saved-pill")
        .h(look.space(HEADER_PILL_HEIGHT))
        .px(look.space(SPACE_LG))
        .gap(look.space(SPACE_LG))
        .center_y()
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .child(
            text(strings::saved_pill(name))
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_primary),
        )
        .child(show);
    row()
        .pin(
            Some(0.0),
            None,
            Some(0.0),
            Some(below_pt + look.space(JUMP_INSET)),
        )
        .justify_center()
        .child(pill)
}
