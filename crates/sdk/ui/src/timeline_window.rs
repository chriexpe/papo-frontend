//! Independent timeline bounds and pure item projection; the SDK owns canonical history, its
//! message order and read points.

use imbl::Vector;
use papo_sdk::{AccountId, HistoryOrder, HistorySnapshot, Message, MessageKey, ReadState, Uuid};
use std::{collections::BTreeMap, sync::Arc};

use crate::{
    content_cache::{self, ContentCacheKey},
    message_content::MessageContent,
    timeline::TimelineError,
    timeline_item::{ItemIdentity, TimelineDirection, TimelineItem, UtcOffsetProvider},
};

/// Inclusive boundaries survive deletion of their boundary messages. A live window extends
/// only its newest bound; a focused window never grows when another view paginates.
pub(crate) struct Bounds {
    pub lower: Option<MessageKey>,
    pub upper: Option<MessageKey>,
    pub live: bool,
}

impl Bounds {
    pub fn contains(&self, point: MessageKey) -> bool {
        self.lower.is_some_and(|lower| point >= lower)
            && (self.live || self.upper.is_some_and(|upper| point <= upper))
    }
}

#[cfg(test)]
/// Projects only this window without cross-window `MessageContent` sharing.
pub(crate) fn project(
    raw: &HistorySnapshot,
    bounds: &Bounds,
    read: Option<&ReadState>,
    loading: [bool; 2],
    offset: &dyn UtcOffsetProvider,
    previous: &Vector<Arc<TimelineItem>>,
) -> Result<Vector<Arc<TimelineItem>>, TimelineError> {
    project_with_cache(raw, bounds, read, loading, offset, previous, None)
}

/// Projects only this window, sharing parsed `MessageContent` `Arc`s within `(AccountId, channel)`.
pub(crate) fn project_with_cache(
    raw: &HistorySnapshot,
    bounds: &Bounds,
    read: Option<&ReadState>,
    loading: [bool; 2],
    offset: &dyn UtcOffsetProvider,
    previous: &Vector<Arc<TimelineItem>>,
    cache_scope: Option<(AccountId, Uuid)>,
) -> Result<Vector<Arc<TimelineItem>>, TimelineError> {
    let reusable: BTreeMap<_, _> = previous
        .iter()
        .map(|item| (item.identity(), item))
        .collect();
    let mut values = Vec::new();
    let mut gaps = BTreeMap::<MessageKey, Vec<_>>::new();
    for gap in &raw.gaps {
        let point = MessageKey::from_cursor(&gap.cursor)?;
        if bounds.contains(point) {
            gaps.entry(point).or_default().push(gap);
        }
    }
    if loading[0] {
        values.push(TimelineItem::Loading(TimelineDirection::Older));
    }
    let mut previous_day = None;
    let coverage = raw.coverage()?;
    // Inclusive bounds: from the first message at or after `lower` through `upper`.
    let start = match bounds.lower {
        Some(lower) => coverage.range(None, Some(lower))?.messages.end,
        None => raw.messages.len(),
    };
    let end = match bounds.upper {
        _ if bounds.live => raw.messages.len(),
        Some(upper) => coverage.range(Some(upper), None)?.messages.start,
        None => 0,
    };
    for message in &raw.messages.skip(start.min(end)).take(end - start.min(end)) {
        let point = MessageKey::from_message(message)?;
        if !bounds.contains(point) {
            continue;
        }
        if let Some(boundaries) = gaps.remove(&point) {
            for gap in boundaries
                .iter()
                .filter(|gap| gap.order == HistoryOrder::Desc)
            {
                values.push(TimelineItem::Gap((*gap).clone()));
            }
            for gap in boundaries
                .iter()
                .filter(|gap| gap.order == HistoryOrder::Asc)
            {
                gaps.entry(point).or_default().push(*gap);
            }
        }
        let day = (i128::from(point.created_at_us)
            + i128::from(offset.offset_at(point.created_at_us).seconds_east()) * 1_000_000)
            .div_euclid(86_400_000_000);
        let day = i64::try_from(day).map_err(|_| TimelineError::InvalidCursor)?;
        if previous_day != Some(day) {
            values.push(TimelineItem::DateSeparator {
                day,
                before: point.message_id,
            });
            previous_day = Some(day);
        }
        if read.and_then(|read| read.first_unread) == Some(point.message_id) {
            values.push(TimelineItem::UnreadMarker {
                before: point.message_id,
            });
        }
        let content = resolve_content(&reusable, cache_scope, point.message_id, message);
        values.push(TimelineItem::Message {
            id: point.message_id,
            message: Arc::clone(message),
            content,
        });
        if let Some(boundaries) = gaps.remove(&point) {
            values.extend(
                boundaries
                    .into_iter()
                    .map(|gap| TimelineItem::Gap(gap.clone())),
            );
        }
    }
    // A deleted boundary is still a real gap, even if its cursor no longer has a message row.
    for (point, boundaries) in gaps {
        let index = values
            .iter()
            .position(|item| match item {
                TimelineItem::Message { message, .. } => {
                    MessageKey::from_message(message).is_ok_and(|current| current > point)
                }
                _ => false,
            })
            .unwrap_or(values.len());
        for gap in boundaries.into_iter().rev() {
            values.insert(index, TimelineItem::Gap(gap.clone()));
        }
    }
    if loading[1] {
        values.push(TimelineItem::Loading(TimelineDirection::Newer));
    }
    Ok(values
        .into_iter()
        .map(|item| {
            reusable
                .get(&item.identity())
                .filter(|old| old.as_ref() == &item)
                .map_or_else(|| Arc::new(item), |old| Arc::clone(old))
        })
        .collect())
}

fn resolve_content(
    reusable: &BTreeMap<ItemIdentity, &Arc<TimelineItem>>,
    cache_scope: Option<(AccountId, Uuid)>,
    id: Uuid,
    message: &Arc<Message>,
) -> Arc<MessageContent> {
    let key = cache_scope.map(|(account, channel)| ContentCacheKey {
        account,
        channel,
        message: id,
    });
    if let Some(old_item) = reusable.get(&ItemIdentity::Message(id))
        && let TimelineItem::Message {
            message: old_message,
            content: old_content,
            ..
        } = old_item.as_ref()
    {
        if Arc::ptr_eq(old_message, message) {
            return Arc::clone(old_content);
        }
        if content_cache::same_revision(old_message, message) {
            if let Some(key) = key {
                content_cache::refresh_revision(key, message, old_content);
            }
            return Arc::clone(old_content);
        }
    }
    content_cache::get_or_parse(key, message)
}

#[cfg(test)]
mod tests;
