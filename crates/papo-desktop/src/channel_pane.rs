//! Draws the channel pane (`docs/design.md` §3, §8): the selected server's
//! pill (its card, ⇄ for its accounts, ⚙), its text and voice channels, and
//! the account pill at the foot. Pure: reads the navigation state.
//!
//! Owns the pane's look and its keyboard and accessibility: the channels are
//! one list the arrows move through, with the selected one as its Tab stop.
//! What a channel shows is D3's. An unread row is bold with a dot at its
//! end, and mentions take the dot's place as a red badge.

use std::collections::HashMap;

use papo_sdk::ServerId;
use rune::{Color, El, Icon, Image, ScrollSpec, TextStyle, col, el, icon, row, text};

use crate::account_switch;
use crate::badges::{Badge, mention_pill};
use crate::design::{
    CHANNEL_ROW_HEIGHT_PT, CORNER_MARKER_OVERHANG_PT, FOCUS_RING_PT, Look, PANE_PILL_HEIGHT,
    PILL_AVATAR_PT, PILL_BUTTON_PT, PILL_ICON_PT, RADIUS_CONTROL, RADIUS_PILL, ROW_ICON_PT,
    SIDEBAR_WIDTH_PT, SPACE_LG, SPACE_MD, SPACE_SM, Surfaces, TYPE_BODY, TYPE_BODY_MEDIUM,
    TYPE_BODY_STRONG, TYPE_CAPTION, TYPE_CAPTION_MEDIUM, TYPE_TITLE, UNREAD_DOT_PT,
};
use crate::icons::{GEAR, HASH, SPEAKER, SWITCH};
use crate::navigation::Navigation;
use crate::rail::initial;
use crate::server_list::{AccountEntry, ChannelEntry, ServerEntry};
use crate::shell::Shell;
use crate::strings;

/// The element key of the server card, which its menu grows from.
pub(crate) const SERVER_CARD_KEY: &str = "server-card";
/// The element key of the server card's ⚙.
pub(crate) const SERVER_OPTIONS_KEY: &str = "server-options";
/// The element key of the server pill's ⇄, which opens its accounts.
pub(crate) const SWITCH_ACCOUNT_KEY: &str = "switch-account";

/// The element key of a channel's row.
pub(crate) fn channel_key(channel: &str) -> String {
    format!("channel-{channel}")
}

/// The pane for the selected server; empty when there is none.
pub(crate) fn view(nav: &Navigation, icons: &HashMap<ServerId, Image>, look: &Look) -> El {
    let pane = col()
        .key("channel-pane")
        .role("navigation")
        .label(strings::channels_label())
        .w(SIDEBAR_WIDTH_PT)
        .h_full()
        .padding(look.space(SPACE_MD))
        .gap(look.space(SPACE_SM))
        .chrome_fill(&look.palette.sidebar_fill);
    let Some(server) = nav.selected() else {
        return pane;
    };
    let accounts_open = nav.accounts_open();
    let server_row = server_row(
        server,
        icons.get(&server.id),
        accounts_open,
        accounts_badge(nav, server),
        look,
    )
    .key(SERVER_CARD_KEY);
    let (head, grown) = if accounts_open {
        // The pill grows over the channels; the spacer holds its place, so
        // they stay where they were.
        let spacer = el().w_full().h(look.space(PANE_PILL_HEIGHT));
        (
            spacer,
            Some(account_switch::grown(server, server_row, look)),
        )
    } else {
        (chrome(server_row, look), None)
    };
    let mut pane = pane
        .child(head)
        .child(channels(server, nav.selected_channel(), look));
    if let Some(account) = &server.account {
        pane = pane.child(account_pill(account, None, look));
    }
    // Last, so it draws over the channels and the account pill.
    if let Some(grown) = grown {
        pane = pane.child(grown);
    }
    pane
}

/// The server pill's row: its card (the avatar and names), ⇄ and ⚙.
fn server_row(
    server: &ServerEntry,
    image: Option<&Image>,
    accounts_open: bool,
    badge: Badge,
    look: &Look,
) -> El {
    let mut names = col().grow(1.0).min_w(0.0).child(
        text(server.name.as_str())
            .font(look.font(TYPE_TITLE))
            .color(look.palette.text_primary)
            .max_lines(1),
    );
    if let Some(owner) = &server.owner {
        names = names.child(
            text(strings::server_owner(owner))
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted)
                .max_lines(1),
        );
    }
    let options = pill_button(SERVER_OPTIONS_KEY, &GEAR, look)
        .label(strings::server_options())
        .on_press_tx(|shell: &mut Shell, tx| shell.open_server_menu(tx));
    pill_row(look)
        .child(avatar(&server.name, image, look))
        .child(names)
        .child(switch_button(
            SWITCH_ACCOUNT_KEY,
            badge,
            accounts_open,
            Shell::toggle_accounts,
            look,
        ))
        .child(options)
}

/// What the accounts on `server` have unread, all of them together: the
/// marker on its ⇄.
fn accounts_badge(nav: &Navigation, server: &ServerEntry) -> Badge {
    let badges = &nav.servers().account_badges;
    Badge::sum(
        server
            .accounts
            .iter()
            .filter_map(|account| badges.get(&account.id).copied()),
    )
}

/// ⇄ (`key`): opens accounts and closes them again while they show,
/// carrying `badge`, what those accounts have unread, on its corner.
pub(crate) fn switch_button(
    key: &'static str,
    badge: Badge,
    open: bool,
    toggle: fn(&mut Shell, &mut rune::Tx),
    look: &Look,
) -> El {
    let mut button = pill_button(key, &SWITCH, look)
        .label(badge.label(strings::switch_account()))
        .expanded(open)
        .on_press_tx(toggle);
    if open {
        button = button.fill(look.palette.surface_selected);
    } else if let Some(marker) = corner_marker(badge, look) {
        button = button.child(marker);
    }
    button
}

/// What a badge shows on a button's top-right corner: the mention count,
/// ringed in the pill's surface, else the unread dot.
fn corner_marker(badge: Badge, look: &Look) -> Option<El> {
    let pinned = |marker: El| {
        marker.pin(
            None,
            Some(-CORNER_MARKER_OVERHANG_PT),
            Some(-CORNER_MARKER_OVERHANG_PT),
            None,
        )
    };
    if let Some(count) = badge.mentions() {
        return Some(pinned(mention_pill(
            count,
            Some(look.palette.surface_pill),
            look,
        )));
    }
    badge.is_unread().then(|| {
        pinned(
            el().size(UNREAD_DOT_PT, UNREAD_DOT_PT)
                .radius(UNREAD_DOT_PT / 2.0)
                .fill(look.palette.text_primary)
                .a11y_hidden(),
        )
    })
}

/// A square icon button on a pill: ⇄ or ⚙.
fn pill_button(key: &'static str, glyph: &'static Icon, look: &Look) -> El {
    row()
        .key(key)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        .radius(look.radius(RADIUS_CONTROL))
        .hover(look.palette.surface_hover)
        .role("button")
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .child(icon(glyph, PILL_ICON_PT).color(look.palette.text_secondary))
}

/// The account pill: the account's avatar, name and handle, then
/// `trailing` at its end (Direct's ⇄; a server's settings later).
pub(crate) fn account_pill(account: &AccountEntry, trailing: Option<El>, look: &Look) -> El {
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
    chrome(
        pill_row(look)
            .key("account")
            .child(avatar(&account.name, None, look))
            .child(names)
            .children(trailing),
        look,
    )
}

/// A pill's row: both pills are one height (`docs/design.md` §8).
pub(crate) fn pill_row(look: &Look) -> El {
    row()
        .w_full()
        .h(look.space(PANE_PILL_HEIGHT))
        .px(look.space(SPACE_LG))
        .gap(look.space(SPACE_MD))
        .center_y()
}

/// A floating pill's surface around `content`.
pub(crate) fn chrome(content: El, look: &Look) -> El {
    content.radius(look.radius(RADIUS_PILL)).pill_surface(look)
}

/// An image, or the first letter of `name` on a neutral tile.
pub(crate) fn avatar(name: &str, image: Option<&Image>, look: &Look) -> El {
    let tile = el()
        .size(PILL_AVATAR_PT, PILL_AVATAR_PT)
        .radius(look.radius(RADIUS_CONTROL))
        .a11y_hidden();
    match image {
        Some(image) => tile.pixels(image.pixels()),
        None => tile.center().fill(look.palette.surface_hover).child(
            text(initial(name))
                .font(look.font(TYPE_CAPTION_MEDIUM))
                .color(look.palette.text_secondary),
        ),
    }
}

/// Both groups, as one list the arrows move through; it scrolls when the
/// server has more channels than fit.
fn channels(server: &ServerEntry, selected: Option<&str>, look: &Look) -> El {
    let mut list = col()
        .key("channels")
        .w_full()
        .role("listbox")
        .label(strings::channels_label())
        .focus_group();
    let groups: [(&str, &[ChannelEntry], &'static Icon); 2] = [
        (strings::text_channels(), &server.text_channels, &HASH),
        (strings::voice_channels(), &server.voice_channels, &SPEAKER),
    ];
    for (label, entries, mark) in groups {
        if entries.is_empty() {
            continue;
        }
        let rows = entries.iter().map(|channel| {
            let selected = selected == Some(channel.id.as_str());
            channel_row(channel, mark, selected, look)
        });
        list = list.child(
            col()
                .w_full()
                .role("group")
                .label(label)
                .child(group_label(label, look))
                .children(rows),
        );
    }
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        // A scroll here is work outside a header panel; it scrolls as usual.
        .on_wheel(|shell: &mut Shell, _wheel, _tx| {
            shell.worked_outside();
            false
        })
        .child(list)
}

/// A group's label, as the catalog writes it (in capitals); the group
/// itself carries it for assistive technology.
fn group_label(label: &str, look: &Look) -> El {
    el().pt(look.space(SPACE_LG))
        .pb(look.space(SPACE_SM))
        .px(look.space(SPACE_MD))
        .child(
            text(label)
                .font(look.font(TYPE_CAPTION_MEDIUM))
                .color(look.palette.text_muted)
                .a11y_hidden(),
        )
}

fn channel_row(channel: &ChannelEntry, mark: &'static Icon, selected: bool, look: &Look) -> El {
    let id = channel.id.clone();
    let (style, color) = row_name(channel.badge, selected, look);
    let mark_color = if selected {
        look.palette.accent
    } else {
        look.palette.text_muted
    };
    let mut row_el = row()
        .key(channel_key(&channel.id))
        .w_full()
        .h(CHANNEL_ROW_HEIGHT_PT)
        .px(look.space(SPACE_MD))
        .gap(look.space(SPACE_SM))
        .center_y()
        .radius(look.radius(RADIUS_CONTROL))
        .hover(look.palette.surface_hover)
        .role("option")
        .label(channel.badge.label(&channel.name))
        .focusable(true)
        .focus_selected(selected)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(move |shell: &mut Shell, tx| shell.select_channel(&id, tx))
        .child(icon(mark, ROW_ICON_PT).color(mark_color).a11y_hidden())
        .child(
            text(channel.name.as_str())
                .font(style)
                .color(color)
                .max_lines(1)
                .grow(1.0)
                .min_w(0.0),
        );
    if selected {
        row_el = row_el.fill(look.palette.surface_selected);
    }
    row_el.children(row_badge(channel.badge, look))
}

/// How a channel's or a conversation's name reads on its row: unread wins
/// the weight (600), selection keeps its colours (500, primary), else
/// regular in `text.secondary`.
pub(crate) fn row_name(badge: Badge, selected: bool, look: &Look) -> (TextStyle, Color) {
    if badge.is_unread() {
        (look.font(TYPE_BODY_STRONG), look.palette.text_primary)
    } else if selected {
        (look.font(TYPE_BODY_MEDIUM), look.palette.text_primary)
    } else {
        (look.font(TYPE_BODY), look.palette.text_secondary)
    }
}

/// A row's end: the mention pill, else the unread dot, else nothing.
pub(crate) fn row_badge(badge: Badge, look: &Look) -> Option<El> {
    if let Some(count) = badge.mentions() {
        return Some(mention_pill(count, None, look));
    }
    badge.is_unread().then(|| {
        el().size(UNREAD_DOT_PT, UNREAD_DOT_PT)
            .radius(UNREAD_DOT_PT / 2.0)
            .fill(look.palette.text_primary)
            .a11y_hidden()
    })
}
