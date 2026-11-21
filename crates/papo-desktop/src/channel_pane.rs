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
use rune::{El, Icon, Image, ScrollSpec, col, el, icon, row, text};

use crate::account_switch;
use crate::badges::mention_pill;
use crate::design::{
    ACCOUNT_COUNT_OVERHANG_PT, ACCOUNT_COUNT_PT, CHANNEL_ICON_PT, CHANNEL_ROW_HEIGHT_PT,
    FOCUS_RING_PT, PANE_PILL_HEIGHT_PT, PILL_AVATAR_PT, PILL_BUTTON_PT, PILL_ICON_PT,
    PILL_OUTLINE_PT, Palette, RADIUS_CONTROL_PT, RADIUS_PILL_PT, SIDEBAR_WIDTH_PT, SPACE_LG_PT,
    SPACE_MD_PT, SPACE_SM_PT, TYPE_BODY, TYPE_BODY_MEDIUM, TYPE_BODY_STRONG, TYPE_CAPTION,
    TYPE_CAPTION_MEDIUM, TYPE_COUNT, TYPE_TITLE, UNREAD_DOT_PT,
};
use crate::icons::{GEAR, HASH, SPEAKER, SWAP};
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
pub(crate) fn view(nav: &Navigation, icons: &HashMap<ServerId, Image>, palette: &Palette) -> El {
    let pane = col()
        .key("channel-pane")
        .w(SIDEBAR_WIDTH_PT)
        .h_full()
        .padding(SPACE_MD_PT)
        .gap(SPACE_SM_PT)
        .fill(palette.surface_sidebar);
    let Some(server) = nav.selected() else {
        return pane;
    };
    let accounts_open = nav.accounts_open();
    let server_row =
        server_row(server, icons.get(&server.id), accounts_open, palette).key(SERVER_CARD_KEY);
    let (head, grown) = if accounts_open {
        // The pill grows over the channels; the spacer holds its place, so
        // they stay where they were.
        let spacer = el().w_full().h(PANE_PILL_HEIGHT_PT);
        (
            spacer,
            Some(account_switch::grown(server, server_row, palette)),
        )
    } else {
        (chrome(server_row, palette), None)
    };
    let mut pane = pane
        .child(head)
        .child(channels(server, nav.selected_channel(), palette));
    if let Some(account) = &server.account {
        pane = pane.child(account_pill(account, palette));
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
    palette: &Palette,
) -> El {
    let mut names = col().grow(1.0).min_w(0.0).child(
        text(server.name.as_str())
            .font(TYPE_TITLE)
            .color(palette.text_primary)
            .max_lines(1),
    );
    if let Some(owner) = &server.owner {
        names = names.child(
            text(strings::server_owner(owner))
                .font(TYPE_CAPTION)
                .color(palette.text_muted)
                .max_lines(1),
        );
    }
    let options = pill_button(SERVER_OPTIONS_KEY, &GEAR, palette)
        .label(strings::server_options())
        .on_press_tx(|shell: &mut Shell, tx| shell.open_server_menu(tx));
    pill_row()
        .child(avatar(&server.name, image, palette))
        .child(names)
        .child(switch_button(server, accounts_open, palette))
        .child(options)
}

/// ⇄: opens the server's accounts, and closes them again while they show.
/// With two or more accounts it carries their count.
fn switch_button(server: &ServerEntry, accounts_open: bool, palette: &Palette) -> El {
    let count = server.accounts.len();
    let label = if count > 1 {
        let spoken = strings::account_count(u64::try_from(count).unwrap_or(u64::MAX));
        strings::badge_labelled(strings::switch_account(), &spoken)
    } else {
        strings::switch_account().to_owned()
    };
    let mut button = pill_button(SWITCH_ACCOUNT_KEY, &SWAP, palette)
        .label(label)
        .expanded(accounts_open)
        .on_press_tx(|shell: &mut Shell, tx| shell.toggle_accounts(tx));
    if accounts_open {
        button = button.fill(palette.surface_selected);
    } else if count > 1 {
        button = button.child(account_count(count, palette));
    }
    button
}

/// The accent badge on ⇄'s top-right corner.
fn account_count(count: usize, palette: &Palette) -> El {
    row()
        .pin(
            None,
            Some(-ACCOUNT_COUNT_OVERHANG_PT),
            Some(-ACCOUNT_COUNT_OVERHANG_PT),
            None,
        )
        .min_w(ACCOUNT_COUNT_PT)
        .h(ACCOUNT_COUNT_PT)
        .px(SPACE_SM_PT / 2.0)
        .center()
        .radius(ACCOUNT_COUNT_PT / 2.0)
        .fill(palette.accent)
        .a11y_hidden()
        .child(
            text(count.to_string())
                .font(TYPE_COUNT)
                .color(palette.accent_on),
        )
}

/// A square icon button on a pill: ⇄ or ⚙.
fn pill_button(key: &'static str, glyph: &'static Icon, palette: &Palette) -> El {
    row()
        .key(key)
        .size(PILL_BUTTON_PT, PILL_BUTTON_PT)
        .center()
        .radius(RADIUS_CONTROL_PT)
        .hover(palette.surface_hover)
        .role("button")
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
        .child(icon(glyph, PILL_ICON_PT).color(palette.text_secondary))
}

fn account_pill(account: &AccountEntry, palette: &Palette) -> El {
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
    chrome(
        pill_row()
            .key("account")
            .child(avatar(&account.name, None, palette))
            .child(names),
        palette,
    )
}

/// A pill's row: both pills are one height (`docs/design.md` §8).
fn pill_row() -> El {
    row()
        .w_full()
        .h(PANE_PILL_HEIGHT_PT)
        .px(SPACE_LG_PT)
        .gap(SPACE_MD_PT)
        .center_y()
}

/// A floating pill's surface around `content`.
fn chrome(content: El, palette: &Palette) -> El {
    content
        .radius(RADIUS_PILL_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
}

/// An image, or the first letter of `name` on a neutral tile.
pub(crate) fn avatar(name: &str, image: Option<&Image>, palette: &Palette) -> El {
    let tile = el()
        .size(PILL_AVATAR_PT, PILL_AVATAR_PT)
        .radius(RADIUS_CONTROL_PT)
        .a11y_hidden();
    match image {
        Some(image) => tile.pixels(image.pixels()),
        None => tile.center().fill(palette.surface_hover).child(
            text(initial(name))
                .font(TYPE_CAPTION_MEDIUM)
                .color(palette.text_secondary),
        ),
    }
}

/// Both groups, as one list the arrows move through; it scrolls when the
/// server has more channels than fit.
fn channels(server: &ServerEntry, selected: Option<&str>, palette: &Palette) -> El {
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
            channel_row(channel, mark, selected, palette)
        });
        list = list.child(
            col()
                .w_full()
                .role("group")
                .label(label)
                .child(group_label(label, palette))
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
        .child(list)
}

/// A group's label, as the catalog writes it (in capitals); the group
/// itself carries it for assistive technology.
fn group_label(label: &str, palette: &Palette) -> El {
    el().pt(SPACE_LG_PT).pb(SPACE_SM_PT).px(SPACE_MD_PT).child(
        text(label)
            .font(TYPE_CAPTION_MEDIUM)
            .color(palette.text_muted)
            .a11y_hidden(),
    )
}

fn channel_row(
    channel: &ChannelEntry,
    mark: &'static Icon,
    selected: bool,
    palette: &Palette,
) -> El {
    let id = channel.id.clone();
    let unread = channel.badge.is_unread();
    // Unread wins the weight; selection keeps its colours.
    let style = if unread {
        TYPE_BODY_STRONG
    } else if selected {
        TYPE_BODY_MEDIUM
    } else {
        TYPE_BODY
    };
    let color = if unread || selected {
        palette.text_primary
    } else {
        palette.text_secondary
    };
    let mark_color = if selected {
        palette.accent
    } else {
        palette.text_muted
    };
    let mut row_el = row()
        .key(channel_key(&channel.id))
        .w_full()
        .h(CHANNEL_ROW_HEIGHT_PT)
        .px(SPACE_MD_PT)
        .gap(SPACE_SM_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .hover(palette.surface_hover)
        .role("option")
        .label(channel.badge.label(&channel.name))
        .focusable(true)
        .focus_selected(selected)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
        .on_press_tx(move |shell: &mut Shell, tx| shell.select_channel(&id, tx))
        .child(icon(mark, CHANNEL_ICON_PT).color(mark_color).a11y_hidden())
        .child(
            text(channel.name.as_str())
                .font(style)
                .color(color)
                .max_lines(1)
                .grow(1.0)
                .min_w(0.0),
        );
    // The mention pill takes the unread dot's place.
    if let Some(count) = channel.badge.mentions() {
        row_el = row_el.child(mention_pill(count, None, palette));
    } else if unread {
        row_el = row_el.child(
            el().size(UNREAD_DOT_PT, UNREAD_DOT_PT)
                .radius(UNREAD_DOT_PT / 2.0)
                .fill(palette.text_primary)
                .a11y_hidden(),
        );
    }
    if selected {
        row_el = row_el.fill(palette.surface_selected);
    }
    row_el
}
