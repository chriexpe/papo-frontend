//! A message's row at each density (`docs/design.md` §5 and §6): a lead
//! column (the avatar, or Extra compact's time gutter) and one content
//! column holding the header and the text. The distance above the row is
//! the between-group or within-group spacing; the row's own padding is the
//! same either way, so every message is its own row with its own identity.
//!
//! Bodies are plain text here; rich content is D3b's. The author's name
//! comes from the account's members, a bridged author's from the message.

use papo_sdk::{Message, MessageKey, ResourceSnapshot, Uuid};
use papo_sdk_ui::TimelineItem;
use rune::{El, col, el, memo, row, text};

use super::OpenTimeline;
use super::grouping::continues;
use super::rows::{MESSAGE_PART, message_key};
use super::view::{Style, TimelineHost};
use crate::design::{
    Density, Lead, MESSAGE_BLOCK_GAP_PT, MESSAGE_HEADER_GAP_PT, MESSAGE_NAME_GAP_PT,
    MESSAGE_ROW_INSET_PT, Palette, RADIUS_AVATAR_PT, TYPE_BODY, TYPE_BODY_STRONG, TYPE_CAPTION,
    TYPE_CAPTION_MEDIUM,
};
use crate::rail::initial;
use crate::strings;

/// What a message row shows, read from the message and its neighbour.
#[derive(Hash)]
struct Parts<'a> {
    name: String,
    time: String,
    body: &'a str,
    /// It continues the group above: no header, the within-group distance.
    continued: bool,
    /// Extra compact shows the time once per minute.
    show_time: bool,
}

/// Message `id`'s row. It reports where its top is, for the anchor.
pub(super) fn message_row<S: TimelineHost>(
    open: &OpenTimeline,
    id: Uuid,
    message: &Message,
    previous: Option<&TimelineItem>,
    style: &Style,
) -> El {
    let layout = style.layout;
    let continued = layout.grouped && continues(previous, message);
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
        continued,
        show_time,
    };
    let key = open.key;
    let version = (style.theme, layout, &parts, &message.edited_at);
    memo(message_key(id), version, || {
        build(&parts, style, |message| {
            message.on_layout_tx(move |host: &mut S, rect, tx| {
                host.row_moved(key, id, [rect[1], rect[1] + rect[3]], tx);
                tx.live_only();
            })
        })
    })
}

/// The row: the distance above it, then the message itself, which
/// `report` makes report its top. The message's top, not the row's, is the
/// anchor, so a change of the distance above leaves the text in place.
fn build(parts: &Parts<'_>, style: &Style, report: impl FnOnce(El) -> El) -> El {
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
    let body = text(parts.body.to_owned())
        .key("body")
        .font(TYPE_BODY)
        .color(palette.text_primary)
        .wrap()
        .w_full();
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
    let message = row()
        .key(MESSAGE_PART)
        .w_full()
        .px(MESSAGE_ROW_INSET_PT)
        .py(spacing.row_padding_pt)
        .gap(spacing.lead_gap_pt)
        .items_start()
        .child(lead)
        .child(content.key("content").grow(1.0).min_w(0.0));
    col().w_full().pt(above_pt).child(report(message))
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
fn author_name(message: &Message, resources: Option<&ResourceSnapshot>) -> String {
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
fn created_us(message: &Message) -> Option<i64> {
    MessageKey::from_message(message)
        .ok()
        .map(|key| key.created_at_us)
}
