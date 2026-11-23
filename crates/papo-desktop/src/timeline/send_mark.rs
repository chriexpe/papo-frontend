//! The mark an unsent message shows in its time's place (`docs/design.md`
//! §3, Your unsent messages): a muted clock while it waits, a danger ⚠ when
//! it failed or may not have arrived. The ⚠ is a button; its menu (Retry,
//! Discard) grows from it, opened by the host. The mark never changes the
//! row's height.

use papo_sdk::Uuid;
use rune::{El, icon, row};

use super::TimelineKey;
use super::echoes::SendState;
use super::view::TimelineHost;
use crate::design::{FOCUS_RING_PT, Look, RADIUS_CONTROL, SEND_STATE_ICON_PT};
use crate::icons::{CLOCK, WARNING};
use crate::strings;

/// The element key of the mark of the unsent message `local`: unique in the
/// window, so its menu can grow from it.
pub(crate) fn mark_key(local: Uuid) -> String {
    format!("send-state-{local}")
}

/// The mark for `state` on the unsent message `local` of timeline `key`.
pub(super) fn send_mark<S: TimelineHost>(
    key: TimelineKey,
    local: Uuid,
    state: SendState,
    look: &Look,
) -> El {
    let mark = row()
        .key(mark_key(local))
        .center()
        .radius(look.radius(RADIUS_CONTROL));
    match state {
        SendState::Pending => mark
            .role("img")
            .label(strings::outgoing_pending())
            .child(icon(&CLOCK, SEND_STATE_ICON_PT).color(look.palette.text_muted)),
        SendState::Failed | SendState::Unknown => {
            let words = if state == SendState::Failed {
                strings::outgoing_failed()
            } else {
                strings::outgoing_unknown()
            };
            mark.role("button")
                .label(strings::outgoing_options(words))
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press(move |host: &mut S| host.send_state_pressed(key, local, state))
                .child(icon(&WARNING, SEND_STATE_ICON_PT).color(look.palette.danger))
        }
    }
}
