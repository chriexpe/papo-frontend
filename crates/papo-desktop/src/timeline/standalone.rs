//! A message on its own (`docs/design.md` §10, StandaloneMessage): outside
//! the conversation (the delete confirmation, pins, later search), with
//! its header always shown and no grouping, its time with its date ("Today,
//! 21:34"), and its body from the conversation's own renderer.

use papo_sdk::{Message, Uuid};
use papo_sdk_ui::TimelineItem;
use papo_sdk_ui::message_content::MessageContent;
use rune::{El, col, row, text};

use super::OpenTimeline;
use super::content::{self, Context, Doc, Draw};
use super::local_time::day_label;
use super::message_row::{author_name, avatar, created_us};
use super::view::{Style, TimelineHost};
use crate::design::{
    Density, Lead, MESSAGE_BLOCK_GAP_PT, MESSAGE_HEADER_GAP_PT, TYPE_BODY_STRONG, TYPE_CAPTION,
};
use crate::strings;

/// Message `id` of `open` on its own; none when the conversation does not
/// show it.
pub(crate) fn standalone<S: TimelineHost>(
    open: &OpenTimeline,
    id: Uuid,
    style: &Style,
) -> Option<El> {
    let (message, content) = open.items.iter().find_map(|item| match item.as_ref() {
        TimelineItem::Message {
            id: current,
            message,
            content,
            ..
        } if *current == id => Some((message, content)),
        _ => None,
    })?;
    Some(draw::<S>(open, id, message, content, style))
}

/// `message` on its own, from its own content (a pin the conversation
/// may not hold: pins draw from the SDK's pin list).
pub(crate) fn standalone_message<S: TimelineHost>(
    open: &OpenTimeline,
    id: Uuid,
    message: &Message,
    content: &MessageContent,
    style: &Style,
) -> El {
    draw::<S>(open, id, message, content, style)
}

fn draw<S: TimelineHost>(
    open: &OpenTimeline,
    id: Uuid,
    message: &Message,
    content: &MessageContent,
    style: &Style,
) -> El {
    let palette = style.palette;
    let spacing = style.layout.density.spacing();
    let name = author_name(message, open.resources.as_deref());
    let time = created_us(message).map_or_else(String::new, |at_us| {
        let day = style.time.day_of(at_us);
        if day >= style.today - 1 {
            strings::date_time(&day_label(day, style.today), &style.time.time_label(at_us))
        } else {
            style.time.moment_label(at_us, style.today)
        }
    });
    let doc = Doc::new(
        content,
        &Context {
            resources: open.resources.as_deref(),
            revealed: open.revealed(id),
            jumbo: style.layout.density != Density::ExtraCompact,
        },
    );
    let draw = Draw {
        palette: &palette,
        key: open.key,
        message: id,
        copied: None,
        edited: None,
    };
    // Extra compact has no avatar in the conversation; on its own, a
    // message keeps Normal's.
    let lead_pt = [spacing.lead, Density::Normal.spacing().lead]
        .into_iter()
        .find_map(|lead| match lead {
            Lead::Avatar(size_pt) => Some(size_pt),
            Lead::Gutter(_) => None,
        })
        .unwrap_or_default();
    row()
        .key(format!("standalone-{id}"))
        .w_full()
        .gap(spacing.lead_gap_pt)
        .items_start()
        .child(avatar(&name, lead_pt, &palette))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(MESSAGE_BLOCK_GAP_PT)
                .child(
                    row()
                        .gap(MESSAGE_HEADER_GAP_PT)
                        .items_end()
                        .child(
                            text(name)
                                .font(TYPE_BODY_STRONG)
                                .color(palette.text_primary),
                        )
                        .child(text(time).font(TYPE_CAPTION).color(palette.text_muted)),
                )
                .child(content::body::<S>(&doc, &draw)),
        )
}
