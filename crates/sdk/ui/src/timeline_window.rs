//! Independent timeline bounds and pure item projection; the SDK owns canonical history and read points.

use imbl::Vector;
use papo_sdk::{HistoryCursor, HistoryOrder, HistorySnapshot, ReadState, Uuid};
use std::{collections::BTreeMap, sync::Arc};

use crate::{
    timeline::TimelineError,
    timeline_item::{TimelineDirection, TimelineItem, UtcOffsetProvider},
};

pub(crate) type Key = (i64, Uuid);

pub(crate) fn key(cursor: &HistoryCursor) -> Result<Key, TimelineError> {
    let created_at_us = chrono::DateTime::parse_from_rfc3339(&cursor.created_at)
        .map_err(|_| TimelineError::InvalidCursor)?
        .timestamp_micros();
    let id = cursor
        .id
        .parse()
        .map_err(|_| TimelineError::InvalidCursor)?;
    Ok((created_at_us, id))
}

/// Inclusive boundaries survive deletion of their boundary messages. A live window extends
/// only its newest bound; a focused window never grows when another view paginates.
pub(crate) struct Bounds {
    pub lower: Option<Key>,
    pub upper: Option<Key>,
    pub live: bool,
}

impl Bounds {
    pub fn contains(&self, point: Key) -> bool {
        self.lower.is_some_and(|lower| point >= lower)
            && (self.live || self.upper.is_some_and(|upper| point <= upper))
    }
}

/// Projects only this window. Every retained item is looked up by identity, not list position.
pub(crate) fn project(
    raw: &HistorySnapshot,
    bounds: &Bounds,
    read: Option<&ReadState>,
    loading: [bool; 2],
    offset: &dyn UtcOffsetProvider,
    previous: &Vector<Arc<TimelineItem>>,
) -> Result<Vector<Arc<TimelineItem>>, TimelineError> {
    let reusable: BTreeMap<_, _> = previous
        .iter()
        .map(|item| (item.identity(), item))
        .collect();
    let mut values = Vec::new();
    let mut gaps = BTreeMap::<Key, Vec<_>>::new();
    for gap in &raw.gaps {
        let point = key(&gap.cursor)?;
        if bounds.contains(point) {
            gaps.entry(point).or_default().push(gap);
        }
    }
    if loading[0] {
        values.push(TimelineItem::Loading(TimelineDirection::Older));
    }
    let mut previous_day = None;
    for message in &raw.messages {
        let cursor = message.cursor().ok_or(TimelineError::InvalidCursor)?;
        let point = key(&cursor)?;
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
        let day = (i128::from(point.0)
            + i128::from(offset.offset_at(point.0).seconds_east()) * 1_000_000)
            .div_euclid(86_400_000_000);
        let day = i64::try_from(day).map_err(|_| TimelineError::InvalidCursor)?;
        if previous_day != Some(day) {
            values.push(TimelineItem::DateSeparator {
                day,
                before: point.1,
            });
            previous_day = Some(day);
        }
        if read.and_then(|read| read.first_unread) == Some(point.1) {
            values.push(TimelineItem::UnreadMarker { before: point.1 });
        }
        values.push(TimelineItem::Message {
            id: point.1,
            message: Arc::clone(message),
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
                TimelineItem::Message { message, .. } => message
                    .cursor()
                    .and_then(|cursor| key(&cursor).ok())
                    .is_some_and(|current| current > point),
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

#[cfg(test)]
mod tests;
