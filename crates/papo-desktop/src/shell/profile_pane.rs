//! A direct conversation's profile pane (`docs/design.md` §10, Direct
//! messages): the person's avatar with presence, name, @handle, About,
//! their roles in that server and "Member since", beside the conversation
//! at the members pane's width. The person button in the actions pill or
//! Ctrl+U shows or hides it, remembered for the window.
//!
//! Owns the toggle and the pane; its content is drawn as the profile card
//! draws it, and no picture is drawn yet.

use papo_sdk_ui::{display_name, person, requested_profile};
use rune::{El, Tx, col, icon, row};

use super::Shell;
use super::profile_card::{Header, sections};
use crate::design::{
    FOCUS_RING_PT, MEMBERS_WIDTH_PT, PILL_BUTTON_PT, PILL_ICON_PT, Palette, RADIUS_CONTROL_PT,
    SPACE_MD_PT,
};
use crate::icons::USER;
use crate::strings;

/// The element key of the actions pill's person button.
pub(crate) const PROFILE_BUTTON_KEY: &str = "profile-button";

impl Shell {
    /// Shows or hides the picked conversation's profile pane, asking for
    /// the person's profile when it shows.
    pub(crate) fn toggle_dm_profile(&mut self, tx: &mut Tx) {
        if self.nav.picked_dm().is_none() {
            return;
        }
        self.worked_outside();
        self.dm_profile = !self.dm_profile;
        if self.dm_profile
            && let (Some(key), Some(picked)) = (self.wanted_direct_timeline(), self.nav.picked_dm())
        {
            self.load_profile(key, picked.user);
        }
        tx.focus(PROFILE_BUTTON_KEY, false);
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
        .on_press_tx(Shell::toggle_dm_profile)
        .child(icon(&USER, PILL_ICON_PT).color(tone))
}

/// The pane, while a picked conversation shows it.
pub(super) fn pane(shell: &Shell, palette: &Palette) -> Option<El> {
    if !shell.dm_profile {
        return None;
    }
    let picked = shell.nav.picked_dm()?;
    let key = shell.wanted_direct_timeline()?;
    let resources = shell
        .shown_timeline(key)
        .and_then(|open| open.resources.as_deref())?;
    let member = person(resources, picked.user);
    let name = display_name(member.nickname.as_deref(), member.username.as_deref())
        .or(display_name(
            picked.nickname.as_deref(),
            picked.username.as_deref(),
        ))
        .unwrap_or(strings::someone())
        .to_owned();
    let people = shell.people(key.account);
    let profile = requested_profile(resources, picked.user);
    let since = profile
        .as_ref()
        .and_then(|profile| profile.member_since.as_deref())
        .and_then(|instant| shell.local_time.full_date(instant));
    let about = profile
        .as_ref()
        .and_then(|profile| profile.description.as_deref())
        .filter(|about| !about.trim().is_empty());
    let roles: Vec<&str> = member
        .roles
        .iter()
        .filter_map(|role| role.name.as_deref())
        .collect();
    let server = shell
        .nav
        .servers()
        .get(key.account.server())
        .map(|server| server.name.clone())
        .unwrap_or_default();
    let header = Header {
        name: &name,
        username: member.username.as_deref(),
        presence: people.and_then(|people| people.presence_mark(picked.user)),
        status: people.and_then(|people| people.status_phrase(picked.user)),
    };
    Some(
        col()
            .key("profile-pane")
            .w(MEMBERS_WIDTH_PT)
            .h_full()
            .pb(SPACE_MD_PT)
            .fill(palette.surface_sidebar)
            .role("complementary")
            .label(strings::direct_profile())
            .children(header.view(palette))
            .children(sections(
                &strings::direct_roles_in(&server),
                &roles,
                about,
                since.as_deref(),
                palette,
            )),
    )
}
