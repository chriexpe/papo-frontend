//! The accounts on the selected server (`docs/design.md` §8): the server pill
//! grown downward from its ⇄, listing every account saved on the server with
//! the shown one ticked, then Add account. Direct's Accounts pill grows the
//! same list upward for every server at once, each account with its unread
//! badge (the SDK keeps every account's session live). Picking a row makes
//! that account its server's active one; the SDK keeps each account's
//! session and cache apart.
//!
//! Owns the grown pill's sections, their keys, keyboard and accessibility.
//! The pill's own row is `channel_pane`'s; switching and adding are the
//! shell's.

use papo_sdk::AccountId;
use rune::{El, col, el, icon, row, text};

use crate::badges::Badge;
use crate::channel_pane::{account_pill, avatar, row_badge, switch_button};
use crate::design::{
    FOCUS_RING_PT, Look, PANE_EDGE_PT, PANE_PILL_HEIGHT, PILL_OUTLINE_PT, RADIUS_CONTROL,
    RADIUS_PILL, ROW_ICON_PT, SPACE_MD, SPACE_SM, TYPE_BODY, TYPE_BODY_MEDIUM, TYPE_CAPTION,
    TYPE_CAPTION_MEDIUM,
};
use crate::icons::{CHECK, PLUS};
use crate::server_list::{AccountEntry, ServerEntry, ServerList};
use crate::shell::Shell;
use crate::strings;

/// The element key of the grown pill.
pub(crate) const ACCOUNTS_KEY: &str = "accounts";
/// The element key of Add account.
pub(crate) const ADD_ACCOUNT_KEY: &str = "add-account";
/// The element key of Direct's Accounts pill.
pub(crate) const DIRECT_ACCOUNTS_KEY: &str = "direct-accounts";
/// The element key of Direct's accounts, grown from its pill.
pub(crate) const DIRECT_ACCOUNTS_LIST_KEY: &str = "direct-accounts-list";

/// The element key of an account's row: unique across servers, as
/// Direct lists every server's accounts.
pub(crate) fn account_key(account: AccountId) -> String {
    format!("account-{}-{}", account.server(), account.user())
}

/// The server pill grown: `pill_row` (the pill's own row) on top, then the
/// accounts and Add account. Pinned over the top of the channel pane, so it
/// covers the pill and grows over the channels. Escape or a click outside
/// closes it.
pub(crate) fn grown(server: &ServerEntry, pill_row: El, look: &Look) -> El {
    let shown = server.account.as_ref().map(|account| account.id);
    let rows = server
        .accounts
        .iter()
        .map(|account| account_row(account, Some(account.id) == shown, None, look));
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
        .px(look.space(SPACE_MD))
        .pb(look.space(SPACE_SM))
        .child(
            text(title)
                .font(look.font(TYPE_CAPTION_MEDIUM))
                .color(look.palette.text_muted)
                .max_lines(1)
                .w_full()
                .a11y_hidden(),
        );
    col()
        .key(ACCOUNTS_KEY)
        .pin(
            Some(look.space(SPACE_MD)),
            Some(look.space(SPACE_MD)),
            Some(look.space(SPACE_MD)),
            None,
        )
        .radius(look.radius(RADIUS_PILL))
        .fill(look.palette.surface_pill)
        .border(PILL_OUTLINE_PT, look.palette.line_pill)
        .role("dialog")
        .label(strings::switch_account())
        .modal(true, Shell::close_accounts)
        .child(pill_row)
        .child(divider(look))
        .child(section(look).child(caption).child(accounts))
        .child(divider(look))
        .child(section(look).child(add_account(look)))
}

fn section(look: &Look) -> El {
    col()
        .w_full()
        .padding(look.space(SPACE_SM))
        .py(look.space(SPACE_MD))
}

fn divider(look: &Look) -> El {
    el().w_full().h(PANE_EDGE_PT).fill(look.palette.line_subtle)
}

/// One account: its avatar, name and handle, its `badge` when given, ticked
/// when it is the one its server shows.
fn account_row(account: &AccountEntry, shown: bool, badge: Option<Badge>, look: &Look) -> El {
    let id = account.id;
    let names = col()
        .grow(1.0)
        .min_w(0.0)
        .child(
            text(account.name.as_str())
                .font(look.font(TYPE_BODY_MEDIUM))
                .color(look.palette.text_primary)
                .max_lines(1),
        )
        .child(
            text(strings::account_handle(&account.username))
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted)
                .max_lines(1),
        );
    let spoken = strings::account_row(&account.name, &account.username);
    let label = badge.map_or_else(|| spoken.clone(), |badge| badge.label(&spoken));
    let mut line = choice(account_key(id), look)
        .role("radio")
        .label(label)
        .checked(shown)
        .focus_selected(shown)
        .on_press_tx(move |shell: &mut Shell, tx| shell.pick_account(id, tx))
        .child(avatar(&account.name, None, look))
        .child(names)
        .children(badge.and_then(|badge| row_badge(badge, look)));
    if shown {
        line = line.child(icon(&CHECK, ROW_ICON_PT).color(look.palette.accent));
    }
    line
}

fn add_account(look: &Look) -> El {
    choice(ADD_ACCOUNT_KEY.to_owned(), look)
        .role("button")
        .label(strings::add_account())
        .on_press(Shell::add_account)
        .child(icon(&PLUS, ROW_ICON_PT).color(look.palette.text_secondary))
        .child(
            text(strings::add_account())
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_primary),
        )
}

/// A pressable row in the grown pill.
fn choice(key: String, look: &Look) -> El {
    row()
        .key(key)
        .w_full()
        .px(look.space(SPACE_MD))
        .py(look.space(SPACE_SM))
        .gap(look.space(SPACE_MD))
        .center_y()
        .radius(look.radius(RADIUS_CONTROL))
        .hover(look.palette.surface_hover)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
}

/// Direct's account pill, at the bottom of the pane: `account` (the one
/// the picked conversation is from, else the first server's), as a
/// server's account pill shows it, with ⇄ at its end carrying what every
/// account on every server has unread. ⇄ opens every server's accounts.
pub(crate) fn direct_pill(
    account: &AccountEntry,
    servers: &ServerList,
    open: bool,
    look: &Look,
) -> El {
    let badge = Badge::sum(servers.account_badges.values().copied());
    let switch = switch_button(
        DIRECT_ACCOUNTS_KEY,
        badge,
        open,
        Shell::toggle_direct_accounts,
        look,
    );
    account_pill(account, Some(switch), look)
}

/// Every server's accounts, grown upward from Direct's account pill over
/// the bottom of the pane: a section per server, each account with its
/// unread badge and the shown one ticked. Escape or a click outside
/// closes it.
pub(crate) fn direct_grown(servers: &ServerList, look: &Look) -> El {
    let mut list = col().w_full();
    for (index, server) in servers.servers.iter().enumerate() {
        if server.accounts.is_empty() {
            continue;
        }
        if index > 0 {
            list = list.child(divider(look));
        }
        let shown = server.account.as_ref().map(|account| account.id);
        let title = strings::accounts_on(&server.name.to_uppercase());
        let rows = server.accounts.iter().map(|account| {
            let badge = servers.account_badges.get(&account.id).copied();
            account_row(account, Some(account.id) == shown, badge, look)
        });
        list = list.child(
            section(look)
                .child(
                    el().w_full()
                        .min_w(0.0)
                        .px(look.space(SPACE_MD))
                        .pb(look.space(SPACE_SM))
                        .child(
                            text(title.as_str())
                                .font(look.font(TYPE_CAPTION_MEDIUM))
                                .color(look.palette.text_muted)
                                .max_lines(1)
                                .w_full()
                                .a11y_hidden(),
                        ),
                )
                .child(
                    col()
                        .w_full()
                        .role("radiogroup")
                        .label(title.as_str())
                        .focus_group()
                        .children(rows),
                ),
        );
    }
    col()
        .key(DIRECT_ACCOUNTS_LIST_KEY)
        .pin(
            Some(look.space(SPACE_MD)),
            None,
            Some(look.space(SPACE_MD)),
            Some(look.space(SPACE_MD) + look.space(PANE_PILL_HEIGHT) + look.space(SPACE_SM)),
        )
        .radius(look.radius(RADIUS_PILL))
        .fill(look.palette.surface_pill)
        .border(PILL_OUTLINE_PT, look.palette.line_pill)
        .role("dialog")
        .label(strings::direct_accounts())
        .modal(true, Shell::close_accounts)
        .child(
            col()
                .w_full()
                .max_h(DIRECT_ACCOUNTS_MAX_PT)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(list),
        )
}

/// The tallest Direct's accounts grow before they scroll, in points.
const DIRECT_ACCOUNTS_MAX_PT: f32 = 420.0;
