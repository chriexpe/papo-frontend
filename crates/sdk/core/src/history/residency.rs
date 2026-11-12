//! Idle canonical-history eviction and payload-free inspection; disk deletion and access policy live elsewhere.

use super::{
    HistoryBudget, HistoryDiagnostics,
    channel::{Channel, HistoryStore},
    model::Entry,
    ordering::Key,
};
use crate::{ClientError, Session};
use std::{
    mem::size_of,
    sync::{Arc, atomic::Ordering},
};

const INDEX_ENTRY_OVERHEAD_BYTES: usize = 64;
const CHUNK_METADATA_ESTIMATE_BYTES: usize = 256;
const UUID_BYTES: usize = 16;

impl HistoryStore {
    /// Called after reading has consumed accepted snapshots. The map lock fences admission;
    /// an extra Arc protects operations between map lookup and their handle counter increment.
    pub(crate) async fn trim(&self, budget: HistoryBudget) {
        let mut channels = self.channels.lock().await;
        let mut idle = Vec::new();
        let mut idle_bytes = 0_usize;
        for (id, channel) in channels.iter() {
            if pinned(channel) {
                continue;
            }
            let bytes = retained_bytes(channel).await;
            idle_bytes = idle_bytes.saturating_add(bytes);
            idle.push((channel.last_access.load(Ordering::Relaxed), *id, bytes));
        }
        idle.sort_unstable_by_key(|(access, id, _)| (*access, *id));
        let mut count = idle.len();
        for (_, id, bytes) in idle {
            if count <= budget.idle_channels && idle_bytes <= budget.idle_bytes {
                break;
            }
            // No capability can be admitted while this map lock is held. Existing receivers
            // can only disappear; a new receiver requires an already-counted channel Arc.
            if channels.get(&id).is_some_and(|channel| !pinned(channel)) {
                channels.remove(&id);
                count -= 1;
                idle_bytes = idle_bytes.saturating_sub(bytes);
                self.counters.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(super) async fn diagnostics(&self) -> HistoryDiagnostics {
        let channels = self.channels.lock().await;
        let mut report = HistoryDiagnostics {
            work: self.counters.snapshot(),
            ..HistoryDiagnostics::default()
        };
        for channel in channels.values() {
            let bytes = retained_bytes(channel).await;
            if pinned(channel) {
                report.pinned_channels += 1;
                report.pinned_bytes = report.pinned_bytes.saturating_add(bytes);
            } else {
                report.idle_channels += 1;
                report.idle_bytes = report.idle_bytes.saturating_add(bytes);
            }
        }
        report
    }
}

fn pinned(channel: &Arc<Channel>) -> bool {
    Arc::strong_count(channel) > 1 || channel.is_open()
}

async fn retained_bytes(channel: &Channel) -> usize {
    let state = channel.state.lock().await;
    let publication = channel
        .publication
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    state
        .encoded_bytes
        .saturating_add(
            state
                .model
                .entries
                .len()
                .saturating_mul(size_of::<Entry>() + INDEX_ENTRY_OVERHEAD_BYTES),
        )
        .saturating_add(
            state
                .model
                .order
                .len()
                .saturating_mul(size_of::<Key>() + INDEX_ENTRY_OVERHEAD_BYTES),
        )
        .saturating_add(
            state
                .model
                .chunks
                .len()
                .saturating_mul(CHUNK_METADATA_ESTIMATE_BYTES),
        )
        .saturating_add(
            state
                .model
                .deletions
                .values()
                .map(|ids| ids.len().saturating_mul(UUID_BYTES) + INDEX_ENTRY_OVERHEAD_BYTES)
                .sum::<usize>(),
        )
        .saturating_add(publication.payload_bytes)
}

impl Session {
    /// Inspect account-local resident history and cumulative work. Does not load cold channels.
    /// Counted bytes are deterministic estimates, not allocator/GPU/process memory measurements.
    /// # Errors
    /// Returns closed-session/client or executor failure.
    pub async fn history_diagnostics(&self) -> Result<HistoryDiagnostics, ClientError> {
        let session = self.clone();
        self.client
            .dispatch(async move {
                if session.is_closed() {
                    return Err(ClientError::Closed);
                }
                Ok(session.inner.history.diagnostics().await)
            })
            .await
    }

    /// Refresh unread state, then enforce configured idle-history limits without deleting disk data.
    /// Active handles, subscriptions and in-flight operations remain pinned. Automatic maintenance
    /// also runs on history changes and once per second while the session is active.
    /// # Errors
    /// Returns closed/superseded session, read-state storage or executor failure.
    pub async fn trim_history(&self) -> Result<HistoryDiagnostics, ClientError> {
        let session = self.clone();
        self.client
            .dispatch(async move {
                crate::reading::reconcile::refresh(&session).await?;
                Ok(session.inner.history.diagnostics().await)
            })
            .await
    }
}

#[cfg(test)]
mod tests;
