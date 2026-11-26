//! A message on its own (`docs/design.md` §10, StandaloneMessage): outside
//! the conversation (the delete confirmation, pins, search results), with
//! its header always shown and no grouping, its time with its date ("Today,
//! 21:34"), and its body from the conversation's own renderer.

use papo_sdk::{Message, Uuid};
use papo_sdk_ui::message_content::MessageContent;
use papo_sdk_ui::{SearchResultItem, TimelineItem};
use rune::{El, col, row, text};

use super::OpenTimeline;
use super::content::{self, Context, Doc, Draw};
use super::local_time::day_label;
use super::message_row::{author_name, avatar, created_us};
use super::row_media::with_media;
use super::view::{Style, TimelineHost};
use crate::design::{
    Density, Lead, MESSAGE_BLOCK_GAP, MESSAGE_HEADER_GAP, TYPE_BODY_STRONG, TYPE_CAPTION,
};
use crate::message_media::received::{self, MediaAt};
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
    Some(draw::<S>(open, id, message, content, style, None))
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
    draw::<S>(open, id, message, content, style, None)
}

/// A search result on its own, from what the hit carries; its author by
/// the username the backend sent when the roster no longer holds them.
pub(crate) fn standalone_hit<S: TimelineHost>(
    open: &OpenTimeline,
    item: &SearchResultItem,
    style: &Style,
) -> El {
    let fallback = item.author_username.as_deref();
    draw::<S>(
        open,
        item.id(),
        &item.message,
        &item.content,
        style,
        fallback,
    )
}

fn draw<S: TimelineHost>(
    open: &OpenTimeline,
    id: Uuid,
    message: &Message,
    content: &MessageContent,
    style: &Style,
    fallback_name: Option<&str>,
) -> El {
    let look = style.look;
    let spacing = look.row_spacing(style.layout);
    let resources = open.resources.as_deref();
    let known = message.external_author.is_some()
        || message
            .author_id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok())
            .zip(resources)
            .is_some_and(|(id, resources)| resources.users.get(id).is_some());
    let name = match fallback_name {
        Some(fallback) if !known && !fallback.is_empty() => fallback.to_owned(),
        _ => author_name(message, resources),
    };
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
    let at = MediaAt {
        surface: style.media.surface,
        account: open.key.account,
        channel: open.key.channel,
        message: id,
        index: 0,
    };
    let attachments = message.attachments.as_deref().unwrap_or_default();
    let media = received::attachments::<S>(at, attachments, &style.media, &look);
    let previews = crate::link_cards::view::previews::<S>(
        at,
        message,
        &style.media,
        crate::link_cards::rule_gap_pt(&look, style.layout.density),
        &look,
    );
    let draw = Draw {
        look: &look,
        key: open.key,
        message: id,
        copied: None,
        edited: None,
        standalone: true,
    };
    // Without avatars in the conversation, a message on its own keeps
    // Normal's.
    let lead_pt = [spacing.lead, look.spacing(Density::Normal).lead]
        .into_iter()
        .find_map(|lead| match lead {
            Lead::Avatar(size_pt) => Some(size_pt),
            Lead::Gutter(_) | Lead::None => None,
        })
        .unwrap_or_default();
    row()
        .key(format!("standalone-{id}"))
        .w_full()
        .gap(spacing.lead_gap_pt)
        .items_start()
        .child(avatar(&name, lead_pt, &look, |tile| tile))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(look.space(MESSAGE_BLOCK_GAP))
                .child(
                    row()
                        .gap(look.space(MESSAGE_HEADER_GAP))
                        .items_end()
                        .child(
                            text(name)
                                .font(look.font(TYPE_BODY_STRONG))
                                .color(look.palette.text_primary),
                        )
                        .child(
                            text(time)
                                .font(look.font(TYPE_CAPTION))
                                .color(look.palette.text_muted),
                        ),
                )
                .child(with_media(
                    content::body::<S>(&doc, &draw),
                    [previews, media],
                    &look,
                )),
        )
}
