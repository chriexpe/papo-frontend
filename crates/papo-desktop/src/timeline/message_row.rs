//! A message's row at each density (`docs/design.md` §5 and §6): a lead
//! column (the avatar, or Extra compact's time gutter) and one content
//! column holding the header and the text. The distance above the row is
//! the between-group or within-group spacing; the row's own padding is the
//! same either way, so every message is its own row with its own identity.
//!
//! The body is `content`'s. A message that mentions the reader (or
//! everyone) has the mention background and the amber bar on its left. The
//! author's name comes from the account's members, a bridged author's from
//! the message. A reading boundary (`boundary`) is drawn over the distance
//! above the row; the new-messages divider also starts a new group.

use papo_sdk::mentions::{contains_everyone_mention, contains_user_mention};
use papo_sdk::{Message, MessageKey, ResourceSnapshot, Uuid, parse_message_timestamp_us};
use papo_sdk_ui::TimelineItem;
use papo_sdk_ui::message_content::MessageContent;
use rune::{El, col, el, memo, row, text};

use super::OpenTimeline;
use super::boundary::boundary as boundary_line;
use super::content::{self, Context, Doc, Draw, Edited, mention_names};
use super::grouping::continues;
use super::local_time::LocalTime;
use super::reading::Boundary;
use super::rows::{MESSAGE_PART, message_key};
use super::view::{Style, TimelineHost};
use crate::design::{
    Density, Lead, MENTION_BAR_PT, MENTION_ROW_RADIUS_PT, MESSAGE_BLOCK_GAP_PT,
    MESSAGE_HEADER_GAP_PT, MESSAGE_NAME_GAP_PT, MESSAGE_ROW_INSET_PT, Palette, RADIUS_AVATAR_PT,
    TYPE_BODY_STRONG, TYPE_CAPTION, TYPE_CAPTION_MEDIUM,
};
use crate::rail::initial;
use crate::strings;

/// What a message row shows, read from the message and its neighbour.
#[derive(Hash)]
struct Parts<'a> {
    name: String,
    time: String,
    /// The message's source, which its content is parsed from: the content
    /// changes with it.
    body: &'a str,
    /// The names its mentions show.
    mentions: Vec<Option<String>>,
    /// It continues the group above: no header, the within-group distance.
    continued: bool,
    /// Extra compact shows the time once per minute.
    show_time: bool,
    /// It mentions the reader or everyone.
    mentions_me: bool,
    /// When it was edited, as the wire says it.
    edited_at: Option<&'a str>,
    /// The reading boundary over the distance above it.
    boundary: Option<Boundary>,
}

/// Message `id`'s row. It reports where its top is, for the anchor.
/// `previous` is the item drawn above it.
pub(super) fn message_row<S: TimelineHost>(
    open: &OpenTimeline,
    id: Uuid,
    message: &Message,
    content: &MessageContent,
    previous: Option<&TimelineItem>,
    style: &Style,
) -> El {
    let layout = style.layout;
    let boundary = open.reading.boundary(id, style.hourglass);
    // Only the divider, held for the visit, splits a group: the block's
    // edges move as the SDK covers what is read, and a split that moved
    // with them would move rows.
    let splits = boundary == Some(Boundary::NewMessages);
    let continued = layout.grouped && !splits && continues(previous, message);
    let at_us = created_us(message);
    let show_time = match previous {
        Some(TimelineItem::Message {
            message: previous, ..
        }) if layout.density == Density::ExtraCompact => {
            let minute = |at_us: Option<i64>| at_us.map(|at_us| style.time.minute_of(at_us));
            minute(created_us(previous)) != minute(at_us)
        }
        _ => true,
    };
    let parts = Parts {
        name: author_name(message, open.resources.as_deref()),
        time: at_us.map_or_else(String::new, |at_us| style.time.time_label(at_us)),
        body: message.content.as_deref().unwrap_or_default(),
        mentions: mention_names(content, open.resources.as_deref()),
        continued,
        show_time,
        mentions_me: mentions(message, open.key.account.user()),
        edited_at: message.edited_at.as_deref(),
        boundary,
    };
    let key = open.key;
    let revealed = open.revealed(id);
    let copied = open.copied(id);
    // The key too: two accounts on one server can show the same message,
    // and the row's handlers name their timeline.
    let version = (key, style.theme, layout, &parts, revealed, copied);
    memo(message_key(id), version, || {
        let doc = Doc::new(
            content,
            &Context {
                resources: open.resources.as_deref(),
                revealed,
                jumbo: layout.density != Density::ExtraCompact,
            },
        );
        let draw = Draw {
            palette: &style.palette,
            key,
            message: id,
            copied,
            edited: parts.edited_at.map(|at| Edited {
                hint: edited_hint(at, &style.time),
            }),
        };
        let body = content::body::<S>(&doc, &draw);
        build(&parts, body, style, |message| {
            message
                .on_layout_tx(move |host: &mut S, rect, tx| {
                    host.row_moved(key, id, [rect[1], rect[1] + rect[3]], tx);
                    tx.live_only();
                })
                // Ctrl+A selects the message under the pointer.
                .on_hover_change(move |host: &mut S, hovered, tx| {
                    host.message_hovered(key, id, hovered);
                    tx.live_only();
                })
        })
    })
}

/// The "(edited)" marker's hint: when, or nothing to add when the time
/// does not read.
fn edited_hint(edited_at: &str, time: &LocalTime) -> Option<String> {
    parse_message_timestamp_us(edited_at)
        .ok()
        .map(|at_us| time.edited_label(at_us))
}

/// Whether `message` mentions `me` or everyone, as the server's
/// notifications decide it.
fn mentions(message: &Message, me: Uuid) -> bool {
    let Some(body) = message.content.as_deref() else {
        return false;
    };
    contains_user_mention(body, me) || contains_everyone_mention(body)
}

/// The row: the distance above it, then the message itself, which
/// `report` makes report its top. The message's top, not the row's, is the
/// anchor, so a change of the distance above leaves the text in place.
fn build(parts: &Parts<'_>, body: El, style: &Style, report: impl FnOnce(El) -> El) -> El {
    let palette = style.palette;
    let spacing = style.layout.density.spacing();
    let above_pt = if parts.continued {
        spacing.within_group_pt
    } else {
        spacing.between_groups_pt
    };
    let name = || {
        text(parts.name.clone())
            .key("name")
            .font(TYPE_BODY_STRONG)
            .color(palette.text_primary)
    };
    let time = || {
        text(parts.time.clone())
            .key("time")
            .font(TYPE_CAPTION)
            .color(palette.text_muted)
    };
    let words = matches!(spacing.lead, Lead::Avatar(_));
    let (lead, content) = match spacing.lead {
        Lead::Avatar(size_pt) => {
            let lead = if parts.continued {
                el().key("lead").w(size_pt)
            } else {
                avatar(&parts.name, size_pt, &palette)
            };
            let mut content = col().gap(MESSAGE_BLOCK_GAP_PT);
            if !parts.continued {
                content = content.child(
                    row()
                        .key("header")
                        .gap(MESSAGE_HEADER_GAP_PT)
                        .items_end()
                        .child(name())
                        .child(time()),
                );
            }
            (lead, content.child(body))
        }
        Lead::Gutter(width_pt) => {
            let mut lead = el().key("lead").w(width_pt);
            if parts.show_time {
                lead = lead.child(time());
            }
            let content = if style.layout.stacked {
                let mut stack = col().gap(MESSAGE_BLOCK_GAP_PT);
                if !parts.continued {
                    stack = stack.child(name());
                }
                stack.child(body)
            } else {
                // A continued row keeps the hidden name's width, so its
                // text stays on the column.
                let name = if parts.continued {
                    name().opacity(0.0).a11y_hidden()
                } else {
                    name()
                };
                row()
                    .gap(MESSAGE_NAME_GAP_PT)
                    .items_start()
                    .child(name)
                    .child(col().grow(1.0).min_w(0.0).child(body))
            };
            (lead, content)
        }
    };
    let mut message = row()
        .key(MESSAGE_PART)
        .w_full()
        .px(MESSAGE_ROW_INSET_PT)
        .py(spacing.row_padding_pt)
        .gap(spacing.lead_gap_pt)
        .items_start()
        .child(lead)
        .child(content.key("content").grow(1.0).min_w(0.0));
    if parts.mentions_me {
        message = message
            .radius(MENTION_ROW_RADIUS_PT)
            .fill(palette.mention_row)
            .border_sides([0.0, 0.0, 0.0, MENTION_BAR_PT], palette.mention_bar);
    }
    let row = col().w_full().pt(above_pt).child(report(message));
    match parts.boundary {
        Some(boundary) => row.child(boundary_line(boundary, above_pt, words, &palette)),
        None => row,
    }
}

/// The first letter of `name` on a rounded square.
fn avatar(name: &str, size_pt: f32, palette: &Palette) -> El {
    el().key("lead")
        .size(size_pt, size_pt)
        .radius(RADIUS_AVATAR_PT)
        .center()
        .fill(palette.surface_hover)
        .a11y_hidden()
        .child(
            text(initial(name))
                .font(TYPE_CAPTION_MEDIUM)
                .color(palette.text_secondary),
        )
}

/// The author's name: a member's nickname or username, a bridged author's
/// name, else "Someone".
pub(super) fn author_name(message: &Message, resources: Option<&ResourceSnapshot>) -> String {
    if let Some(external) = &message.external_author {
        return external
            .name
            .clone()
            .unwrap_or_else(|| strings::someone().to_owned());
    }
    let member = message
        .author_id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok())
        .zip(resources)
        .and_then(|(id, resources)| resources.users.get(id));
    member
        .and_then(|user| user.nickname.clone().or_else(|| user.username.clone()))
        .unwrap_or_else(|| strings::someone().to_owned())
}

/// When the message was created, in Unix microseconds.
pub(super) fn created_us(message: &Message) -> Option<i64> {
    MessageKey::from_message(message)
        .ok()
        .map(|key| key.created_at_us)
}
