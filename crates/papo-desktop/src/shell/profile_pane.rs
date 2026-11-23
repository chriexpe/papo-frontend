//! A direct conversation's profile pane (`docs/design.md` §10, Direct
//! messages): the person's avatar with presence, name, @handle, About,
//! their roles in that server, "Member since" and Block / Unblock, beside
//! the conversation at the members pane's width. The person button in the actions pill or
//! Ctrl+U shows or hides it, remembered across launches (`layout_prefs`).
//!
//! Owns the toggle and the pane; its content is the profile card's facts,
//! About first and without the status bubble, and no picture is drawn yet.

use papo_sdk::Uuid;
use rune::{El, col, icon, row, text};

use super::Shell;
use super::profile_card::{Order, PersonFacts, header, sections};
use crate::design::{
    FOCUS_RING_PT, MEMBERS_WIDTH_PT, PILL_BUTTON_PT, PILL_ICON_PT, Palette, RADIUS_CONTROL_PT,
    SPACE_LG_PT, SPACE_MD_PT, SPACE_SM_PT, TYPE_BODY,
};
use crate::icons::{BLOCK, USER};
use crate::strings;
use crate::timeline::TimelineKey;

/// The element key of the actions pill's person button.
pub(crate) const PROFILE_BUTTON_KEY: &str = "profile-button";

impl Shell {
    /// Shows or hides the picked conversation's profile pane, asking for
    /// the person's profile when it shows. The keyboard stays where it
    /// was: Ctrl+U from the composer goes on writing.
    pub(crate) fn toggle_dm_profile(&mut self) {
        if self.nav.picked_dm().is_none() {
            return;
        }
        self.worked_outside();
        let shown = !self.layout.dm_profile();
        self.layout.set_dm_profile(shown);
        if shown
            && let (Some(key), Some(picked)) = (self.wanted_direct_timeline(), self.nav.picked_dm())
        {
            self.load_profile(key, picked.user);
        }
    }
}

/// The actions pill's person button, in the accent while the pane shows.
pub(super) fn button(shown: bool, palette: &Palette) -> El {
    let tone = if shown {
        palette.accent
    } else {
        palette.text_secondary
    };
    row()
        .key(PROFILE_BUTTON_KEY)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        .radius(RADIUS_CONTROL_PT)
        .role("button")
        .label(strings::direct_profile())
        .help(strings::direct_profile())
        .help_keys([strings::shortcut_profile()])
        .expanded(shown)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .hover(palette.surface_hover)
        .on_press(Shell::toggle_dm_profile)
        .child(icon(&USER, PILL_ICON_PT).color(tone))
}

/// The pane, while a picked conversation shows it.
pub(super) fn pane(shell: &Shell, palette: &Palette) -> Option<El> {
    if !shell.layout.dm_profile() {
        return None;
    }
    let picked = shell.nav.picked_dm()?;
    let key = shell.wanted_direct_timeline()?;
    let facts = PersonFacts::of(shell, key, picked.user)?;
    let blocked = shell
        .shown_timeline(key)
        .and_then(|open| open.resources.as_deref())
        .is_some_and(|resources| resources.blocks.contains(picked.user));
    Some(
        col()
            .key("profile-pane")
            .w(MEMBERS_WIDTH_PT)
            .h_full()
            .pb(SPACE_MD_PT)
            .fill(palette.surface_sidebar)
            .role("complementary")
            .label(strings::direct_profile())
            .children(header(&facts, false, palette))
            .children(sections(&facts, Order::Pane, palette))
            .child(block_button(key, picked.user, blocked, palette)),
    )
}

/// Block, or Unblock: at once, each undoing the other.
fn block_button(key: TimelineKey, user: Uuid, blocked: bool, palette: &Palette) -> El {
    let label = if blocked {
        strings::card_unblock()
    } else {
        strings::card_block()
    };
    row()
        .key("pane-block")
        .w_full()
        .mt(SPACE_LG_PT)
        .mx(SPACE_SM_PT)
        .px(SPACE_MD_PT)
        .h(PILL_BUTTON_PT)
        .gap(SPACE_MD_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .hover(palette.surface_hover)
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .on_press(move |shell: &mut Shell| shell.set_blocked(key.account, user, !blocked))
        .child(icon(&BLOCK, PILL_ICON_PT).color(palette.text_secondary))
        .child(
            text(label)
                .font(TYPE_BODY)
                .color(palette.text_primary)
                .a11y_hidden(),
        )
}
