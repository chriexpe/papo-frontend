//! Per-window visibility observers that isolate timeline scroll continuity per view instance.
//! Interval evaluation and storage commits belong to `reading::catch_up_ops`.

use std::sync::{Arc, Weak, atomic::Ordering};

use papo_api::Uuid;

use crate::{
    Generation, ReadingError, Session, VisibilityContinuity, catch_up::VisibleBounds,
    session::SessionInner,
};

struct ObserverSlot {
    id: u64,
    inner: Weak<SessionInner>,
}

impl Drop for ObserverSlot {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            let mut map = inner
                .reading
                .observers
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            map.remove(&self.id);
        }
    }
}

/// Per-window visibility tracker isolating scroll continuity across concurrent timeline windows.
#[derive(Clone)]
pub struct ReadingObserver {
    session: Session,
    slot: Arc<ObserverSlot>,
}

impl std::fmt::Debug for ReadingObserver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadingObserver")
            .field("id", &self.slot.id)
            .finish_non_exhaustive()
    }
}

impl ReadingObserver {
    pub(super) fn new(session: Session) -> Self {
        let id = session
            .inner
            .reading
            .next_observer
            .fetch_add(1, Ordering::Relaxed);
        let slot = Arc::new(ObserverSlot {
            id,
            inner: Arc::downgrade(&session.inner),
        });
        Self { session, slot }
    }

    /// Reports chronological IDs currently visible in this specific timeline window.
    /// Continuity isolates consecutive scrolls inside this window from concurrent windows.
    /// # Errors
    /// Returns history closure, snapshot version, generation, invalid report or storage errors.
    pub async fn report_visible(
        &self,
        channel: Uuid,
        generation: Generation,
        history_version: u64,
        ids: &[Uuid],
        continuity: VisibilityContinuity,
    ) -> Result<(), ReadingError> {
        self.session
            .report_with_observer(
                self.slot.id,
                channel,
                generation,
                history_version,
                ids,
                continuity,
            )
            .await
    }
}

impl super::Reading {
    pub(super) fn observer_last_visible(
        &self,
        observer_id: u64,
        channel: Uuid,
    ) -> Option<VisibleBounds> {
        self.observers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&observer_id)
            .and_then(|channels| channels.get(&channel).copied())
    }

    pub(super) fn set_observer_last_visible(
        &self,
        observer_id: u64,
        channel: Uuid,
        bounds: VisibleBounds,
    ) {
        self.observers
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(observer_id)
            .or_default()
            .insert(channel, bounds);
    }
}
