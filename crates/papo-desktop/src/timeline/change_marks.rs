//! How your edits and deletions in flight draw on their rows
//! (`docs/design.md` §11, by §3's rule that the state takes the time slot):
//! an edit's clock while it saves and ⚠ when it failed, whose menu (Retry,
//! Discard edit) grows from it, opened by the host; and the line under a
//! message whose deletion failed, with Try again and Dismiss.

use papo_sdk::Uuid;
use rune::{El, icon, row, text};

use super::TimelineKey;
use super::changes::ChangeState;
use super::view::TimelineHost;
use crate::design::{
    FOCUS_RING_PT, Palette, RADIUS_CONTROL_PT, SEND_STATE_ICON_PT, SPACE_SM_PT, TYPE_SMALL,
    TYPE_SMALL_MEDIUM,
};
use crate::icons::{CLOCK, WARNING};
use crate::strings;

/// The element key of the mark of the change `local`: unique in the
/// window, so its menu can grow from it.
pub(crate) fn change_mark_key(local: Uuid) -> String {
    format!("change-state-{local}")
}

/// The time slot's mark for an edit in `state`, none for any other state.
pub(super) fn change_mark<S: TimelineHost>(
    key: TimelineKey,
    local: Uuid,
    state: ChangeState,
    palette: &Palette,
) -> Option<El> {
    let mark = row()
        .key(change_mark_key(local))
        .center()
        .radius(RADIUS_CONTROL_PT);
    match state {
        ChangeState::Saving => Some(
            mark.role("img")
                .label(strings::edit_saving())
                .child(icon(&CLOCK, SEND_STATE_ICON_PT).color(palette.text_muted)),
        ),
        ChangeState::EditFailed => Some(
            mark.role("button")
                .label(strings::edit_failed())
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
                .on_press(move |host: &mut S| host.change_mark_pressed(key, local))
                .child(icon(&WARNING, SEND_STATE_ICON_PT).color(palette.danger)),
        ),
        ChangeState::Saved | ChangeState::Deleting | ChangeState::DeleteFailed => None,
    }
}

/// "Couldn't delete the message. Try again · Dismiss", under the message
/// whose deletion `local` failed.
pub(super) fn delete_failed_line<S: TimelineHost>(
    key: TimelineKey,
    local: Uuid,
    palette: &Palette,
) -> El {
    let action = |name: &'static str, label: &'static str| {
        text(label)
            .key(name)
            .font(TYPE_SMALL_MEDIUM)
            .color(palette.link)
            .role("button")
            .focusable(true)
            .radius(RADIUS_CONTROL_PT)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
    };
    row()
        .key("delete-failed")
        .gap(SPACE_SM_PT)
        .center_y()
        .child(
            text(strings::delete_failed())
                .font(TYPE_SMALL)
                .color(palette.danger),
        )
        .child(
            action("delete-retry", strings::delete_try_again())
                .on_press(move |host: &mut S| host.change_retried(key, local)),
        )
        .child(
            text("·")
                .font(TYPE_SMALL)
                .color(palette.text_muted)
                .a11y_hidden(),
        )
        .child(
            action("delete-dismiss", strings::delete_dismiss())
                .on_press(move |host: &mut S| host.change_discarded(key, local)),
        )
}
