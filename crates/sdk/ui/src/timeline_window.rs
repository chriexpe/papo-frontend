//! Independent timeline bounds and pure item projection; the SDK owns canonical history and read points.

use imbl::Vector;
use papo_sdk::{ChronologicalKey, HistoryOrder, HistorySnapshot, ReadState};
use std::{collections::BTreeMap, sync::Arc};

use crate::{
    timeline::TimelineError,
    timeline_item::{TimelineDirection, TimelineItem, UtcOffsetProvider},
};

/// Inclusive boundaries survive deletion of their boundary messages. A live window extends
/// only its newest bound; a focused window never grows when another view paginates.
pub(crate) struct Bounds {
    pub lower: Option<ChronologicalKey>,
    pub upper: Option<ChronologicalKey>,
    pub live: bool,
}

impl Bounds {
    pub fn contains(&self, point: ChronologicalKey) -> bool {
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
    let mut gaps = BTreeMap::<ChronologicalKey, Vec<_>>::new();
    for gap in &raw.gaps {
        let point = ChronologicalKey::from_cursor(&gap.cursor)?;
        if bounds.contains(point) {
            gaps.entry(point).or_default().push(gap);
        }
    }
    if loading[0] {
        values.push(TimelineItem::Loading(TimelineDirection::Older));
    }
    let mut previous_day = None;
    let start = match bounds.lower {
        Some(lower) => raw.range(None, Some(lower))?.indices.end,
        None => raw.messages.len(),
    };
    let end = if bounds.live {
        raw.messages.len()
    } else if let Some(upper) = bounds.upper {
        raw.range(Some(upper), None)?.indices.start
    } else {
        0
    };
    for message in &raw.messages.skip(start.min(end)).take(end - start.min(end)) {
        let point = ChronologicalKey::from_message(message.as_ref())?;
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
        values.push(TimelineItem::Message {
            id: point.message_id,
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
                TimelineItem::Message { message, .. } => {
                    ChronologicalKey::from_message(message).is_ok_and(|current| current > point)
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

#[cfg(test)]
mod tests;
