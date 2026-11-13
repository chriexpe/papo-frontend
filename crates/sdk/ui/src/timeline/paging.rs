//! Window-local pagination and cancellation cleanup; canonical page validation/merging remains in the SDK.

use super::{Owner, Timeline, TimelineDirection, TimelineError, TimelineItem};
use papo_sdk::{Generation, HistoryCursor, HistoryGap, HistoryOrder, HistoryQuery};
use std::sync::Arc;

/// Metadata about one completed page; contents remain in the observable window.
pub struct TimelinePage {
    /// Epoch that accepted the canonical page.
    pub generation: Generation,
    /// Number of messages returned, including any already cached by another window.
    pub message_count: usize,
    /// More history may exist in this direction, including across a gap.
    pub has_more: bool,
}

struct Loading {
    owner: Arc<Owner>,
    index: usize,
}
impl Drop for Loading {
    fn drop(&mut self) {
        let mut state = self.owner.lock();
        if !state.loading[self.index] {
            return;
        }
        state.loading[self.index] = false;
        let _ = state.reproject(&self.owner.config, false);
    }
}

impl Timeline {
    /// Extend only this handle's oldest/newest bound. The other handles' windows are unchanged.
    /// Loading is observable until completion or cancellation; fetching never acknowledges reading.
    /// # Errors
    /// Returns busy, closure, stale generation, transport or cache failures.
    pub async fn paginate(
        &self,
        direction: TimelineDirection,
    ) -> Result<TimelinePage, TimelineError> {
        let cursor = {
            let mut state = self.inner.lock();
            state.refresh(&self.inner.config)?;
            match direction {
                TimelineDirection::Older => state.older_cursor.clone(),
                TimelineDirection::Newer => state.newer_cursor.clone(),
            }
        };
        self.page_at(cursor, direction).await
    }

    /// Fill an explicit gap currently displayed in this window. Its exact cursor is validated
    /// against current items; callers cannot use an unrelated channel's detached cursor here.
    /// # Errors
    /// Returns stale gap, busy, closure, generation, transport or storage failures.
    pub async fn fill_gap(&self, gap: &HistoryGap) -> Result<TimelinePage, TimelineError> {
        {
            let mut state = self.inner.lock();
            state.refresh(&self.inner.config)?;
            let gap_present = state.publisher.items.iter().any(|item| {
                matches!(
                    item.as_ref(),
                    TimelineItem::Gap(current)
                        if current.cursor == gap.cursor && current.order == gap.order
                )
            });
            if !gap_present {
                return Err(TimelineError::StaleSnapshot);
            }
        }
        let direction = if gap.order == HistoryOrder::Desc {
            TimelineDirection::Older
        } else {
            TimelineDirection::Newer
        };
        self.page_at(Some(gap.cursor.clone()), direction).await
    }

    async fn page_at(
        &self,
        cursor: Option<HistoryCursor>,
        direction: TimelineDirection,
    ) -> Result<TimelinePage, TimelineError> {
        let index = usize::from(direction == TimelineDirection::Newer);
        {
            let mut state = self.inner.lock();
            state.refresh(&self.inner.config)?;
            if state.loading[index] {
                return Err(TimelineError::Busy);
            }
            state.loading[index] = true;
            state.reproject(&self.inner.config, false)?;
        }
        let loading = Loading {
            owner: Arc::clone(&self.inner),
            index,
        };
        let page = self
            .inner
            .history
            .page(HistoryQuery {
                cursor: cursor.as_ref(),
                order: direction.order(),
                limit: Some(self.inner.config.page_size),
            })
            .await?;
        {
            let mut state = self.inner.lock();
            state.refresh(&self.inner.config)?;
            for message in &page.messages {
                state.include(&message.cursor().ok_or(TimelineError::InvalidCursor)?)?;
            }
            if let Some(cursor) = &page.next_cursor {
                state.include(cursor)?;
            }
            state.loading[index] = false;
            state.reproject(&self.inner.config, false)?;
        }
        // Prevent a duplicate final publication; the guard still cleans up every early exit.
        drop(loading);
        Ok(TimelinePage {
            generation: page.generation,
            message_count: page.messages.len(),
            has_more: page.has_more,
        })
    }
}
