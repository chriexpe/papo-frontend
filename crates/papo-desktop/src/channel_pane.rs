//! Draws the channel pane (`docs/design.md` §3): the selected server's card
//! pill with ⚙, its text and voice channels, and the account pill at the
//! foot. Pure: reads the navigation state.
//!
//! Owns the pane's look and its keyboard and accessibility: the channels are
//! one list the arrows move through, with the selected one as its Tab stop.
//! What a channel shows is D3's; badges are D1c's, and a row leaves room for
//! one at its end.

use std::collections::HashMap;

use papo_sdk::ServerId;
use rune::{El, Icon, Image, ScrollSpec, col, el, icon, row, text};

use crate::design::{
    CHANNEL_ICON_PT, CHANNEL_ROW_HEIGHT_PT, FOCUS_RING_PT, HEADER_PILL_HEIGHT_PT, PILL_AVATAR_PT,
    PILL_ICON_PT, PILL_OUTLINE_PT, Palette, RADIUS_CONTROL_PT, RADIUS_PILL_PT, SIDEBAR_WIDTH_PT,
    SPACE_LG_PT, SPACE_MD_PT, SPACE_SM_PT, TYPE_BODY, TYPE_BODY_MEDIUM, TYPE_CAPTION,
    TYPE_CAPTION_MEDIUM, TYPE_TITLE,
};
use crate::icons::{GEAR, HASH, SPEAKER};
use crate::navigation::Navigation;
use crate::rail::initial;
use crate::server_list::{AccountEntry, ChannelEntry, ServerEntry};
use crate::shell::Shell;
use crate::strings;

/// The element key of the server card, which its menu grows from.
pub(crate) const SERVER_CARD_KEY: &str = "server-card";
/// The element key of the server card's ⚙.
pub(crate) const SERVER_OPTIONS_KEY: &str = "server-options";

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
    let mut pane = pane
        .child(server_card(server, icons.get(&server.id), palette))
        .child(channels(server, nav.selected_channel(), palette));
    if let Some(account) = &server.account {
        pane = pane.child(account_pill(account, palette));
    }
    pane
}

fn server_card(server: &ServerEntry, image: Option<&Image>, palette: &Palette) -> El {
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
    let options = row()
        .key(SERVER_OPTIONS_KEY)
        .size(PILL_AVATAR_PT, PILL_AVATAR_PT)
        .center()
        .radius(RADIUS_CONTROL_PT)
        .hover(palette.surface_hover)
        .role("button")
        .label(strings::server_options())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
        .on_press(Shell::open_server_menu)
        .child(icon(&GEAR, PILL_ICON_PT).color(palette.text_muted));
    pill(palette)
        .key(SERVER_CARD_KEY)
        .child(avatar(&server.name, image, palette))
        .child(names)
        .child(options)
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
    pill(palette)
        .key("account")
        .child(avatar(&account.name, None, palette))
        .child(names)
}

/// A floating pill in the pane.
fn pill(palette: &Palette) -> El {
    row()
        .w_full()
        .min_h(HEADER_PILL_HEIGHT_PT)
        .px(SPACE_LG_PT)
        .py(SPACE_SM_PT)
        .gap(SPACE_MD_PT)
        .center_y()
        .radius(RADIUS_PILL_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
}

/// An image, or the first letter of `name` on a neutral tile.
fn avatar(name: &str, image: Option<&Image>, palette: &Palette) -> El {
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
    let (style, color, mark_color) = if selected {
        (TYPE_BODY_MEDIUM, palette.text_primary, palette.accent)
    } else {
        (TYPE_BODY, palette.text_secondary, palette.text_muted)
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
        .label(channel.name.as_str())
        .focusable(true)
        .focus_selected(selected)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
        .on_press(move |shell: &mut Shell| shell.select_channel(&id))
        .child(icon(mark, CHANNEL_ICON_PT).color(mark_color).a11y_hidden())
        .child(
            text(channel.name.as_str())
                .font(style)
                .color(color)
                .max_lines(1)
                .grow(1.0)
                .min_w(0.0),
        );
    if selected {
        row_el = row_el.fill(palette.surface_selected);
    }
    row_el
}
