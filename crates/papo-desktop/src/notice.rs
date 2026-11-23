//! The notice pill (`docs/design.md` §14, Refusals): one line saying that
//! something you asked for did not happen, one gap above the composer's
//! text pill and left-aligned with it, floating over the conversation. It
//! goes after [`NOTICE_FOR_S`] unless the pointer rests on it; ✕ closes it.
//! Assistive technology hears it once.
//!
//! Owns what a notice holds and how it is drawn; what calls for one is the
//! shell's.

use std::time::Duration;

use rune::{Align, El, Side, Tx, icon, row, text};

use crate::design::{
    FOCUS_RING_PT, Look, PILL_ICON_PT, RADIUS_CONTROL, RADIUS_PILL, SPACE_LG, SPACE_MD, SPACE_SM,
    Surfaces, TYPE_SMALL,
};
use crate::shell::Shell;
use crate::strings;

/// How long a notice stays, in seconds, while the pointer is not on it.
pub(crate) const NOTICE_FOR_S: f64 = 4.0;
/// The notice pill's height, in points.
const NOTICE_HEIGHT_PT: f32 = 32.0;
/// The element key of the notice pill.
pub(crate) const NOTICE_KEY: &str = "notice";

/// What did not happen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// A reaction you added or removed.
    Reaction,
    /// Pinning a message.
    Pin,
    /// Unpinning a message.
    Unpin,
    /// Closing a direct conversation.
    CloseDm,
    /// Opening a direct conversation.
    OpenDm,
    /// A direct conversation the backend refuses.
    DmRefused,
    Block,
    Unblock,
    Ban,
}

impl Refusal {
    fn words(self) -> &'static str {
        match self {
            Refusal::Reaction => strings::notice_reaction_failed(),
            Refusal::Pin => strings::notice_pin_failed(),
            Refusal::Unpin => strings::notice_unpin_failed(),
            Refusal::CloseDm => strings::notice_dm_close_failed(),
            Refusal::OpenDm => strings::notice_dm_open_failed(),
            Refusal::DmRefused => strings::notice_dm_refused(),
            Refusal::Block => strings::notice_block_failed(),
            Refusal::Unblock => strings::notice_unblock_failed(),
            Refusal::Ban => strings::notice_ban_failed(),
        }
    }
}

/// The notice shown, and until when.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Notice {
    pub(crate) refusal: Refusal,
    /// When it goes, on the window's clock; none while the pointer is on it.
    pub(crate) until_s: Option<f64>,
}

impl Notice {
    /// A notice of `refusal` shown now; asks Rune to wake when it goes.
    pub(crate) fn show(refusal: Refusal, tx: &mut Tx) -> Notice {
        tx.wake_after(Duration::from_secs_f64(NOTICE_FOR_S));
        Notice {
            refusal,
            until_s: Some(tx.clock() + NOTICE_FOR_S),
        }
    }

    /// Whether it has gone by `clock_s`.
    pub(crate) fn gone(&self, clock_s: f64) -> bool {
        self.until_s.is_some_and(|until_s| clock_s >= until_s)
    }

    /// The pointer came onto it (it stays) or left it (it goes
    /// [`NOTICE_FOR_S`] from now).
    pub(crate) fn hovered(&mut self, hovered: bool, tx: &mut Tx) {
        if hovered {
            self.until_s = None;
        } else {
            *self = Notice::show(self.refusal, tx);
        }
    }
}

/// The pill, one gap above the element keyed `above` (the composer's text
/// pill), from its left edge.
pub(crate) fn view(notice: &Notice, above: &str, look: &Look) -> El {
    let close = row()
        .key("notice-close")
        .size(
            NOTICE_HEIGHT_PT - look.space(SPACE_SM),
            NOTICE_HEIGHT_PT - look.space(SPACE_SM),
        )
        .center()
        .radius(look.radius(RADIUS_CONTROL))
        .role("button")
        .label(strings::dismiss())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(Shell::close_notice)
        .child(icon(&crate::icons::X, PILL_ICON_PT).color(look.palette.text_secondary));
    let pill = row()
        .key(NOTICE_KEY)
        .h(NOTICE_HEIGHT_PT)
        .pl(look.space(SPACE_LG))
        .pr(look.space(SPACE_SM) / 2.0)
        .gap(look.space(SPACE_SM))
        .center_y()
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .role("status")
        .live_region(rune::LiveRegion::Polite)
        .on_hover_change(|shell: &mut Shell, hovered, tx| shell.notice_hovered(hovered, tx))
        .child(icon(&crate::icons::WARNING, PILL_ICON_PT).color(look.palette.danger))
        .child(
            text(notice.refusal.words())
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_primary),
        )
        .child(close);
    pill.anchored(above, Side::Top, Align::Start, look.space(SPACE_MD))
}
