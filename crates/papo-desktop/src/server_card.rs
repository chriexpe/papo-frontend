//! The selected server's card (`docs/design.md` §8, "D9 additions"): the
//! server pill grown downward when clicked, showing online and total people,
//! the server address with Copy, per-channel notification modes (All messages,
//! Mentions only, Off) plus All channels, Mark all as read, and Remove server.
//!
//! Owns the grown card's sections, element keys, keyboard and accessibility.
//! The pill's own row is `channel_pane`'s; saving modes, marking read and
//! copying are the shell's (`shell/server_card`).

use papo_sdk::BackendMode;
use rune::{El, ScrollSpec, col, el, icon, row, text};

use crate::design::{
    FOCUS_RING_PT, Look, PANE_EDGE_PT, RADIUS_CONTROL, RADIUS_PILL, ROW_ICON_PT, SPACE_MD,
    SPACE_SM, Surfaces, TYPE_BODY, TYPE_BODY_MEDIUM, TYPE_CAPTION, TYPE_CAPTION_MEDIUM, TYPE_SMALL,
};
use crate::icons::{CHECK, CHEVRON_RIGHT, COPY, HASH, TRASH};
use crate::server_list::{ChannelEntry, ServerEntry};
use crate::shell::Shell;
use crate::strings;

/// The element key of the clickable server pill body that opens the server card.
pub(crate) const SERVER_CARD_OPEN_KEY: &str = "server-card-open";
/// The element key of the grown server card.
pub(crate) const SERVER_CARD_PANEL_KEY: &str = "server-card-panel";
/// The element key of the Copy address button.
pub(crate) const SERVER_CARD_COPY_KEY: &str = "server-card-copy";
/// The element key of the All channels notification mode row.
pub(crate) const SERVER_CARD_ALL_CHANNELS_KEY: &str = "server-card-all-channels";
/// The element key of the Mark all as read button.
pub(crate) const SERVER_CARD_MARK_ALL_KEY: &str = "server-card-mark-all-read";
/// The element key of the Remove server button inside the card.
pub(crate) const SERVER_CARD_REMOVE_KEY: &str = "server-card-remove";

/// Maximum height of the channel notification list inside the server card before scrolling.
const CHANNELS_MAX_PT: f32 = 220.0;

/// The element key of a channel's notification mode row inside the server card.
pub(crate) fn channel_notify_key(channel: &str) -> String {
    format!("server-card-notify-{channel}")
}

/// The three notification modes offered per channel and for All channels (§8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChannelNotifyMode {
    All,
    Mentions,
    Off,
}

impl ChannelNotifyMode {
    /// Every mode in menu order.
    pub(crate) const ALL: [ChannelNotifyMode; 3] = [
        ChannelNotifyMode::All,
        ChannelNotifyMode::Mentions,
        ChannelNotifyMode::Off,
    ];

    /// Maps a backend mode from resources to one of the three known choices.
    pub(crate) fn from_backend(mode: Option<&BackendMode>) -> Option<ChannelNotifyMode> {
        match mode {
            Some(BackendMode::All) => Some(ChannelNotifyMode::All),
            Some(BackendMode::OnlyMentions) => Some(ChannelNotifyMode::Mentions),
            Some(BackendMode::Off) => Some(ChannelNotifyMode::Off),
            Some(BackendMode::Unknown(_)) | None => None,
        }
    }

    /// The wire string sent to `PUT /channels/{id}/users/{user}/settings`.
    pub(crate) fn as_wire(self) -> &'static str {
        match self {
            ChannelNotifyMode::All => "all",
            ChannelNotifyMode::Mentions => "only_mentions",
            ChannelNotifyMode::Off => "off",
        }
    }

    /// The translated display label for this mode.
    pub(crate) fn label(self) -> &'static str {
        match self {
            ChannelNotifyMode::All => strings::server_card_mode_all(),
            ChannelNotifyMode::Mentions => strings::server_card_mode_mentions(),
            ChannelNotifyMode::Off => strings::server_card_mode_off(),
        }
    }
}

/// Transient state shown inside the open server card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CardState {
    /// Live online members reported by the server's activity service.
    pub(crate) online: usize,
    /// Address was just copied to the clipboard.
    pub(crate) copied: bool,
    /// Saving a channel notification mode failed.
    pub(crate) mode_failed: bool,
    /// Mark all as read failed.
    pub(crate) mark_all_failed: bool,
}

/// The server pill grown downward into the server card: `pill_row` on top,
/// then member count and address, per-channel notification modes, and actions.
pub(crate) fn grown(server: &ServerEntry, pill_row: El, state: CardState, look: &Look) -> El {
    let mut card = col()
        .key(SERVER_CARD_PANEL_KEY)
        .pin(
            Some(look.space(SPACE_MD)),
            Some(look.space(SPACE_MD)),
            Some(look.space(SPACE_MD)),
            None,
        )
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .role("dialog")
        .label(server.name.as_str())
        .modal(true, Shell::close_server_card)
        .child(pill_row)
        .child(divider(look))
        .child(info_section(server, state, look));

    if server.account.is_some() && !server.text_channels.is_empty() {
        card =
            card.child(divider(look))
                .child(notifications_section(server, state.mode_failed, look));
    }

    card.child(divider(look))
        .child(actions_section(server, state.mark_all_failed, look))
}

fn info_section(server: &ServerEntry, state: CardState, look: &Look) -> El {
    let mut box_el = section(look).gap(look.space(SPACE_SM));
    let total = server.member_count.max(state.online);
    if server.account.is_some() && total > 0 {
        box_el = box_el.child(
            el().px(look.space(SPACE_MD)).child(
                text(strings::server_card_online_people(state.online, total))
                    .font(look.font(TYPE_CAPTION))
                    .color(look.palette.text_muted)
                    .max_lines(1),
            ),
        );
    }
    let copy_label = if state.copied {
        strings::server_card_copied()
    } else {
        strings::server_card_copy_address()
    };
    let copy_btn = row()
        .key(SERVER_CARD_COPY_KEY)
        .px(look.space(SPACE_SM))
        .py(look.space(SPACE_SM) / 2.0)
        .gap(look.space(SPACE_SM) / 2.0)
        .center_y()
        .radius(look.radius(RADIUS_CONTROL))
        .hover(look.on_pill(look.palette.surface_hover))
        .role("button")
        .label(copy_label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(|shell: &mut Shell, tx| shell.copy_server_address(tx))
        .child(
            icon(if state.copied { &CHECK } else { &COPY }, ROW_ICON_PT).color(if state.copied {
                look.palette.accent
            } else {
                look.palette.text_secondary
            }),
        )
        .children(state.copied.then(|| {
            text(strings::server_card_copied())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.accent)
        }));
    let address_row = row()
        .w_full()
        .px(look.space(SPACE_MD))
        .gap(look.space(SPACE_SM))
        .center_y()
        .child(
            text(server.address.as_str())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_secondary)
                .grow(1.0)
                .min_w(0.0)
                .max_lines(1),
        )
        .child(copy_btn);
    box_el.child(address_row)
}

fn notifications_section(server: &ServerEntry, failed: bool, look: &Look) -> El {
    let title = strings::server_card_notifications();
    let caption = el()
        .w_full()
        .px(look.space(SPACE_MD))
        .pb(look.space(SPACE_SM))
        .child(
            text(title)
                .font(look.font(TYPE_CAPTION_MEDIUM))
                .color(look.palette.text_muted)
                .max_lines(1)
                .a11y_hidden(),
        );
    let server_id = server.id;
    let channel_rows = server
        .text_channels
        .iter()
        .map(|channel| channel_mode_row(server_id, channel, look));
    let all_mode = common_mode(&server.text_channels);
    let all_label = all_mode.map_or("", ChannelNotifyMode::label);
    let all_spoken = if all_label.is_empty() {
        strings::server_card_all_channels().to_owned()
    } else {
        format!("{}, {all_label}", strings::server_card_all_channels())
    };
    let all_row = choice(SERVER_CARD_ALL_CHANNELS_KEY.to_owned(), look)
        .role("button")
        .label(all_spoken)
        .on_press_tx(move |shell: &mut Shell, tx| {
            shell.open_channel_notify_menu(server_id, None, tx);
        })
        .child(
            text(strings::server_card_all_channels())
                .font(look.font(TYPE_BODY_MEDIUM))
                .color(look.palette.text_primary)
                .grow(1.0)
                .min_w(0.0)
                .max_lines(1),
        )
        .children((!all_label.is_empty()).then(|| {
            text(all_label)
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted)
                .max_lines(1)
        }))
        .child(icon(&CHEVRON_RIGHT, ROW_ICON_PT).color(look.palette.text_muted));

    let list = col()
        .w_full()
        .max_h(CHANNELS_MAX_PT)
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .child(
            col()
                .w_full()
                .role("group")
                .label(title)
                .children(channel_rows)
                .child(all_row),
        );
    let mut box_el = section(look).child(caption).child(list);
    if failed {
        box_el = box_el.child(error_line(strings::server_card_mode_failed(), look));
    }
    box_el
}

fn channel_mode_row(server_id: papo_sdk::ServerId, channel: &ChannelEntry, look: &Look) -> El {
    let channel_id = channel.id.clone();
    let mode_label = ChannelNotifyMode::from_backend(channel.notification_mode.as_ref())
        .map_or("", ChannelNotifyMode::label);
    let spoken = if mode_label.is_empty() {
        channel.name.clone()
    } else {
        format!("{}, {mode_label}", channel.name)
    };
    choice(channel_notify_key(&channel.id), look)
        .role("button")
        .label(spoken)
        .on_press_tx(move |shell: &mut Shell, tx| {
            shell.open_channel_notify_menu(server_id, Some(channel_id.as_str()), tx);
        })
        .child(
            icon(&HASH, ROW_ICON_PT)
                .color(look.palette.text_muted)
                .a11y_hidden(),
        )
        .child(
            text(channel.name.as_str())
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_primary)
                .grow(1.0)
                .min_w(0.0)
                .max_lines(1),
        )
        .children((!mode_label.is_empty()).then(|| {
            text(mode_label)
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted)
                .max_lines(1)
        }))
        .child(icon(&CHEVRON_RIGHT, ROW_ICON_PT).color(look.palette.text_muted))
}

fn actions_section(server: &ServerEntry, mark_all_failed: bool, look: &Look) -> El {
    let server_id = server.id;
    let mut box_el = section(look);
    if server.account.is_some() {
        box_el = box_el.child(
            choice(SERVER_CARD_MARK_ALL_KEY.to_owned(), look)
                .role("button")
                .label(strings::server_card_mark_all_read())
                .on_press(move |shell: &mut Shell| shell.mark_all_read(server_id))
                .child(icon(&CHECK, ROW_ICON_PT).color(look.palette.text_secondary))
                .child(
                    text(strings::server_card_mark_all_read())
                        .font(look.font(TYPE_BODY))
                        .color(look.palette.text_primary),
                ),
        );
        if mark_all_failed {
            box_el = box_el.child(error_line(
                strings::server_card_mark_all_read_failed(),
                look,
            ));
        }
    }
    box_el.child(
        choice(SERVER_CARD_REMOVE_KEY.to_owned(), look)
            .role("button")
            .label(strings::remove_server())
            .on_press_tx(move |shell: &mut Shell, tx| {
                shell.ask_remove_server_from_card(server_id, tx);
            })
            .child(icon(&TRASH, ROW_ICON_PT).color(look.palette.danger))
            .child(
                text(strings::remove_server())
                    .font(look.font(TYPE_BODY))
                    .color(look.palette.danger),
            ),
    )
}

/// Returns the mode shared by every text channel on the server, if all agree.
pub(crate) fn common_mode(channels: &[ChannelEntry]) -> Option<ChannelNotifyMode> {
    let first = ChannelNotifyMode::from_backend(channels.first()?.notification_mode.as_ref())?;
    channels
        .iter()
        .all(|channel| {
            ChannelNotifyMode::from_backend(channel.notification_mode.as_ref()) == Some(first)
        })
        .then_some(first)
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

fn error_line(words: &str, look: &Look) -> El {
    el().w_full()
        .px(look.space(SPACE_MD))
        .pt(look.space(SPACE_SM))
        .role("alert")
        .child(
            text(words)
                .font(look.font(TYPE_SMALL))
                .color(look.palette.danger),
        )
}

fn choice(key: String, look: &Look) -> El {
    row()
        .key(key)
        .w_full()
        .px(look.space(SPACE_MD))
        .py(look.space(SPACE_SM))
        .gap(look.space(SPACE_MD))
        .center_y()
        .radius(Look::nested_radius(
            look.radius(RADIUS_PILL),
            look.space(SPACE_SM),
        ))
        .hover(look.on_pill(look.palette.surface_hover))
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
}
