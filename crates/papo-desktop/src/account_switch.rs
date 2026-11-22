//! The accounts on the selected server (`docs/design.md` §8): the server pill
//! grown downward from its ⇄, listing every account saved on the server with
//! the shown one ticked, then Add account. Picking a row makes that account
//! the server's active one; the SDK keeps each account's session and cache
//! apart.
//!
//! Owns the grown pill's sections, their keys, keyboard and accessibility.
//! The pill's own row is `channel_pane`'s; switching and adding are the
//! shell's.

use papo_sdk::AccountId;
use rune::{El, col, el, icon, row, text};

use crate::channel_pane::avatar;
use crate::design::{
    FOCUS_RING_PT, PANE_EDGE_PT, PILL_OUTLINE_PT, Palette, RADIUS_CONTROL_PT, RADIUS_PILL_PT,
    ROW_ICON_PT, SPACE_MD_PT, SPACE_SM_PT, TYPE_BODY, TYPE_BODY_MEDIUM, TYPE_CAPTION,
    TYPE_CAPTION_MEDIUM,
};
use crate::icons::{CHECK, PLUS};
use crate::server_list::{AccountEntry, ServerEntry};
use crate::shell::Shell;
use crate::strings;

/// The element key of the grown pill.
pub(crate) const ACCOUNTS_KEY: &str = "accounts";
/// The element key of Add account.
pub(crate) const ADD_ACCOUNT_KEY: &str = "add-account";

/// The element key of an account's row.
pub(crate) fn account_key(account: AccountId) -> String {
    format!("account-{}", account.user())
}

/// The server pill grown: `pill_row` (the pill's own row) on top, then the
/// accounts and Add account. Pinned over the top of the channel pane, so it
/// covers the pill and grows over the channels. Escape or a click outside
/// closes it.
pub(crate) fn grown(server: &ServerEntry, pill_row: El, palette: &Palette) -> El {
    let shown = server.account.as_ref().map(|account| account.id);
    let rows = server
        .accounts
        .iter()
        .map(|account| account_row(account, Some(account.id) == shown, palette));
    // Written in capitals, as the catalog writes the rest of the caption.
    let title = strings::accounts_on(&server.name.to_uppercase());
    let accounts = col()
        .w_full()
        .role("radiogroup")
        .label(title.as_str())
        .focus_group()
        .children(rows);
    // As wide as the pill, so a long server name ends in an ellipsis.
    let caption = el()
        .w_full()
        .min_w(0.0)
        .px(SPACE_MD_PT)
        .pb(SPACE_SM_PT)
        .child(
            text(title)
                .font(TYPE_CAPTION_MEDIUM)
                .color(palette.text_muted)
                .max_lines(1)
                .w_full()
                .a11y_hidden(),
        );
    col()
        .key(ACCOUNTS_KEY)
        .pin(
            Some(SPACE_MD_PT),
            Some(SPACE_MD_PT),
            Some(SPACE_MD_PT),
            None,
        )
        .radius(RADIUS_PILL_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
        .role("dialog")
        .label(strings::switch_account())
        .modal(true, Shell::close_accounts)
        .child(pill_row)
        .child(divider(palette))
        .child(section().child(caption).child(accounts))
        .child(divider(palette))
        .child(section().child(add_account(palette)))
}

fn section() -> El {
    col().w_full().padding(SPACE_SM_PT).py(SPACE_MD_PT)
}

fn divider(palette: &Palette) -> El {
    el().w_full().h(PANE_EDGE_PT).fill(palette.line_subtle)
}

/// One account: its avatar, name and handle, ticked when it is the one the
/// channel list shows.
fn account_row(account: &AccountEntry, shown: bool, palette: &Palette) -> El {
    let id = account.id;
    let names = col()
        .grow(1.0)
        .min_w(0.0)
        .child(
            text(account.name.as_str())
                .font(TYPE_BODY_MEDIUM)
                .color(palette.text_primary)
                .max_lines(1),
        )
        .child(
            text(strings::account_handle(&account.username))
                .font(TYPE_CAPTION)
                .color(palette.text_muted)
                .max_lines(1),
        );
    let mut line = choice(account_key(id), palette)
        .role("radio")
        .label(strings::account_row(&account.name, &account.username))
        .checked(shown)
        .focus_selected(shown)
        .on_press_tx(move |shell: &mut Shell, tx| shell.pick_account(id, tx))
        .child(avatar(&account.name, None, palette))
        .child(names);
    if shown {
        line = line.child(icon(&CHECK, ROW_ICON_PT).color(palette.accent));
    }
    line
}

fn add_account(palette: &Palette) -> El {
    choice(ADD_ACCOUNT_KEY.to_owned(), palette)
        .role("button")
        .label(strings::add_account())
        .on_press(Shell::add_account)
        .child(icon(&PLUS, ROW_ICON_PT).color(palette.text_secondary))
        .child(
            text(strings::add_account())
                .font(TYPE_BODY)
                .color(palette.text_primary),
        )
}

/// A pressable row in the grown pill.
fn choice(key: String, palette: &Palette) -> El {
    row()
        .key(key)
        .w_full()
        .px(SPACE_MD_PT)
        .py(SPACE_SM_PT)
        .gap(SPACE_MD_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .hover(palette.surface_hover)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
}
