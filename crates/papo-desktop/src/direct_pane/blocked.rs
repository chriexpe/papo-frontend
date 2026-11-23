//! Direct's Blocked view (`docs/design.md` §10, Direct messages): the
//! pill's switch, and the people each shown account blocks, with the server
//! under each and Unblock at the end. Their conversations are not in
//! Direct's list: the backend leaves out conversations blocked in either
//! direction. Pure: reads the shell.
//!
//! Owns the view; blocking is the profile card's (`Shell::set_blocked`).

use papo_sdk::AccountId;
use rune::{El, col, el, icon, row, text};

use crate::design::{
    AVATAR_ROW, DM_ROW_HEIGHT_PT, FOCUS_RING_PT, PILL_BUTTON_PT, PILL_ICON_PT, Palette,
    RADIUS_CONTROL_PT, SPACE_MD_PT, SPACE_SM_PT, TYPE_BODY, TYPE_CAPTION,
};
use crate::icons::BLOCK;
use crate::person_avatar::person_avatar;
use crate::server_list::BlockedPerson;
use crate::shell::Shell;
use crate::strings;

/// The pill's switch between the conversations and the blocked people, in
/// the accent while it shows them.
pub(super) fn blocked_button(shown: bool, palette: &Palette) -> El {
    let tone = if shown {
        palette.accent
    } else {
        palette.text_secondary
    };
    row()
        .key(BLOCKED_BUTTON_KEY)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        .radius(RADIUS_CONTROL_PT)
        .role("button")
        .label(strings::direct_blocked_button())
        .help(strings::direct_blocked_button())
        .expanded(shown)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .hover(palette.surface_hover)
        .on_press(Shell::toggle_direct_blocked)
        .child(icon(&BLOCK, PILL_ICON_PT).color(tone))
}

/// The element key of the pill's Blocked switch.
pub(crate) const BLOCKED_BUTTON_KEY: &str = "direct-blocked";

/// The people each shown account blocks, with the server under each and
/// Unblock at the end. Their conversations are not in the list: the backend
/// leaves out conversations blocked in either direction.
pub(super) fn blocked_list(shell: &Shell, palette: &Palette) -> El {
    let rows: Vec<El> = shell
        .navigation()
        .servers()
        .servers
        .iter()
        .filter_map(|server| Some((server, server.account.as_ref()?)))
        .flat_map(|(server, account)| {
            server
                .blocked
                .iter()
                .map(move |person| blocked_row(account.id, &server.name, person, palette))
        })
        .collect();
    if rows.is_empty() {
        return el().padding(SPACE_MD_PT).child(
            text(strings::direct_blocked_empty())
                .font(TYPE_CAPTION)
                .color(palette.text_muted),
        );
    }
    col()
        .key("blocked-list")
        .w_full()
        .role("list")
        .label(strings::direct_blocked_list())
        .children(rows)
}

fn blocked_row(account: AccountId, server: &str, person: &BlockedPerson, palette: &Palette) -> El {
    let name = person
        .name
        .clone()
        .unwrap_or_else(|| strings::someone().to_owned());
    let user = person.id;
    row()
        .key(format!("blocked-{}-{user}", account.user()))
        .w_full()
        .h(DM_ROW_HEIGHT_PT)
        .px(SPACE_SM_PT)
        .gap(SPACE_MD_PT)
        .center_y()
        .role("listitem")
        .label(strings::direct_title_label(&name, server))
        .child(person_avatar(
            &name,
            AVATAR_ROW,
            None,
            palette.surface_sidebar,
            palette,
        ))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .a11y_hidden()
                .child(
                    text(name.clone())
                        .font(TYPE_BODY)
                        .color(palette.text_secondary)
                        .max_lines(1),
                )
                .child(
                    text(server.to_owned())
                        .font(TYPE_CAPTION)
                        .color(palette.text_muted)
                        .max_lines(1),
                ),
        )
        .child(
            row()
                .key(format!("unblock-{}-{user}", account.user()))
                .px(SPACE_MD_PT)
                .py(SPACE_SM_PT)
                .radius(RADIUS_CONTROL_PT)
                .hover(palette.surface_hover)
                .role("button")
                .label(strings::direct_unblock(&name))
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
                .on_press(move |shell: &mut Shell| shell.set_blocked(account, user, false))
                .child(
                    text(strings::card_unblock())
                        .font(TYPE_CAPTION)
                        .color(palette.link)
                        .a11y_hidden(),
                ),
        )
}
