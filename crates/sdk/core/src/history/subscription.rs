//! Complete raw history snapshots and their bounded publication cell; windows and reading live above it.

use std::sync::Arc;

use papo_api::{HistoryCursor, Uuid, wire::Message};
use tokio::sync::watch;

use crate::Generation;

use super::{HistoryClosedReason, HistoryError, HistoryGap};

/// One proven chronological run, including boundaries whose messages may have been deleted.
#[derive(Clone)]
pub struct HistoryChunk {
    /// Exact oldest boundary, inclusive; never reconstructed from a rounded timestamp.
    pub first: HistoryCursor,
    /// Exact newest boundary, inclusive.
    pub last: HistoryCursor,
    /// No unknown messages precede this run in its validation generation.
    pub oldest_complete: bool,
    /// No unknown messages follow this run in its live-edge generation.
    pub newest_complete: bool,
    /// Epoch that validated this run, or `None` for restored, unvalidated coverage.
    pub generation: Option<Generation>,
    /// Epoch that validated the newest edge, or `None` after restoring from disk.
    pub live_generation: Option<Generation>,
}

/// Complete canonical raw history for one account/channel, not a view window.
/// Messages are unique and ascending by exact `(created_at_us, UUID)`; detached pins are absent.
/// Unknown ranges are explicit, so consumers must not infer continuity from adjacent messages.
#[derive(Clone)]
pub struct HistorySnapshot {
    /// Monotonic publication version within this handle's channel incarnation.
    pub version: u64,
    /// Current connection epoch, including epoch changes with no message changes.
    pub generation: Generation,
    /// Chronological server payloads; unchanged payloads retain their `Arc` identity.
    pub messages: Vec<Arc<Message>>,
    /// Proven runs in ascending boundary order, independent of any consumer's window.
    pub chunks: Vec<HistoryChunk>,
    /// Exact excluded boundaries and directions that can be passed to `page()` to fill holes.
    pub gaps: Vec<HistoryGap>,
    /// A server response proved the entire channel empty in this snapshot's generation.
    pub empty_complete: bool,
}

/// A bounded raw history delivery. Every data delivery is a complete replacement, not a delta.
#[derive(Clone)]
#[non_exhaustive]
pub enum HistoryUpdate {
    /// Replace all raw state. Intermediate versions may coalesce without losing coherence.
    Reset(Arc<HistorySnapshot>),
    /// Terminal, sticky revocation. No subsequent reset can revive this subscription.
    Closed(HistoryClosedReason),
}

/// Atomic initial state plus a one-slot, coalescing watch of complete raw replacements.
/// The initial watch value is marked seen. Call `changed().await`, then `borrow_and_update()`;
/// a version jump is an explicit full reset, never an instruction to replay missing deltas.
/// Producers never await a consumer. Retaining old snapshots is the consumer's memory choice.
/// Clone an update and release the watch borrow before awaiting or calling back into history;
/// retaining a watch borrow can block writers, as with every Tokio watch receiver.
/// On `Closed`, stop consuming; the terminal value remains observable even if its sender drops.
pub struct HistorySubscription {
    /// Snapshot captured atomically with watch registration under canonical history locking.
    pub snapshot: Arc<HistorySnapshot>,
    /// Public bounded watch; every unseen update supersedes all older unseen updates.
    pub updates: watch::Receiver<HistoryUpdate>,
}

impl super::HistoryHandle {
    /// Atomically subscribe to complete canonical raw snapshots, with one-slot coherent resets.
    /// This does not fetch history or move any pagination cursor. Fill an initial empty cache
    /// through `page()`; subsequent accepted pages and live mutations publish under the same lock.
    /// # Errors
    /// Returns typed closure, unverified cache, generation or storage failures.
    pub async fn subscribe(&self) -> Result<HistorySubscription, HistoryError> {
        let generation = self.session.inner.epochs.current().generation;
        let _state = self.channel.state.lock().await;
        self.check(generation)?;
        self.session.inner.epochs.apply(generation, || {
            self.channel.generation_changed(generation)?;
            self.channel.subscribe()
        })?
    }

    /// Linearize a queued visible report with this exact channel incarnation's accepted
    /// publication. The callback must be short, synchronous and contain no commit/fsync;
    /// the FULL writer acknowledges only after its later SQLite commit succeeds.
    pub(crate) fn with_current_snapshot<T>(
        &self,
        generation: Generation,
        version: u64,
        apply: impl FnOnce() -> T,
    ) -> Result<T, HistoryError> {
        self.session.inner.epochs.apply(generation, || {
            let publication = self
                .channel
                .publication
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            publication.check()?;
            let snapshot = publication
                .snapshot
                .as_ref()
                .ok_or(HistoryError::Unverified)?;
            if snapshot.generation != generation || snapshot.version != version {
                return Err(HistoryError::StaleSnapshot);
            }
            Ok(apply())
        })?
    }

    /// Validate a previously reported raw snapshot stamp and return that exact current snapshot.
    /// Consumers resolve reported IDs only from its chronological messages, never detached pins.
    /// Validation is atomic with accepted history publication; callers must fence their own work
    /// against subsequent account/generation changes before acknowledging asynchronous effects.
    /// # Errors
    /// Returns typed closure, superseded generation, or `StaleSnapshot` for an older version.
    pub async fn validate_snapshot(
        &self,
        generation: Generation,
        version: u64,
    ) -> Result<Arc<HistorySnapshot>, HistoryError> {
        let _state = self.channel.state.lock().await;
        self.check(generation)?;
        self.session.inner.epochs.apply(generation, || {
            let subscription = self.channel.subscribe()?;
            if subscription.snapshot.generation != generation
                || subscription.snapshot.version != version
            {
                return Err(HistoryError::StaleSnapshot);
            }
            Ok(subscription.snapshot)
        })?
    }
}

impl super::channel::HistoryStore {
    /// Coalesced wakeups; consumers subscribe before enumerating complete accepted snapshots.
    pub(crate) fn subscribe_changes(&self) -> watch::Receiver<()> {
        self.changes.subscribe()
    }

    pub(crate) async fn snapshots(&self) -> Vec<(Uuid, Arc<HistorySnapshot>)> {
        self.channels
            .lock()
            .await
            .iter()
            .filter_map(|(id, channel)| {
                channel
                    .subscribe()
                    .ok()
                    .map(|subscription| (*id, subscription.snapshot))
            })
            .collect()
    }

    pub(crate) async fn close_all(&self) {
        for channel in self.channels.lock().await.values() {
            channel.close(HistoryClosedReason::AccountClosed);
        }
    }

    pub(crate) async fn generation_changed(
        &self,
        generation: Generation,
    ) -> Result<(), HistoryError> {
        let channels: Vec<_> = self.channels.lock().await.values().cloned().collect();
        for channel in channels {
            match channel.generation_changed(generation) {
                Ok(()) | Err(HistoryError::Closed(_)) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}
