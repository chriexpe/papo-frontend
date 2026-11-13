//! Global physical allocation accounting and ordered cache reclamation; durable state is measured but never evicted.

mod measurement;
mod reclamation;
#[cfg(test)]
mod tests;
mod victims;

use crate::{ClientError, ClientHandle, MediaError};
use std::{sync::Arc, time::Duration};

/// Profile-wide physical disk allocation ceiling, never multiplied by account count.
#[derive(Clone, Copy, Debug)]
pub struct DiskBudget {
    /// All SDK database/sidecar, encoded media and protected outgoing allocated bytes.
    pub bytes: u64,
    /// Minimum interval between automatic pressure checks; explicit trim calls are immediate.
    pub maintenance_interval: Duration,
}
impl Default for DiskBudget {
    fn default() -> Self {
        Self {
            bytes: if cfg!(target_os = "android") {
                256 * 1024 * 1024
            } else {
                1024 * 1024 * 1024
            },
            maintenance_interval: Duration::from_secs(5),
        }
    }
}

/// Actual allocated filesystem bytes and work counters; no message/URL/account identifiers.
#[derive(Clone, Copy, Debug, Default, serde::Serialize)]
pub struct DiskReport {
    /// Configured global ceiling.
    pub budget_bytes: u64,
    /// Physical allocation for databases, sidecars and encoded files, counted once.
    pub allocated_bytes: u64,
    /// Database files plus exact WAL/SHM/journal sidecars, including state.sqlite.
    pub database_bytes: u64,
    /// Shared encoded media files, including staging/orphans.
    pub media_bytes: u64,
    /// Durable outgoing blobs, protected from cache eviction.
    pub outgoing_bytes: u64,
    /// Protected profile lock and filesystem-security migration marker allocation.
    pub profile_bytes: u64,
    /// Actual allocation difference after maintenance, saturating at zero.
    pub reclaimed_bytes: u64,
    /// Number of LRU content files evicted.
    pub media_evictions: u64,
    /// Number of bounded history-prefix eviction transactions.
    pub history_turns: u64,
    /// Largest requested history-row mutation count, including metadata, in a committed retention turn.
    pub max_history_rows_per_turn: usize,
    /// True if a reader blocked a truncating checkpoint during this maintenance burst.
    pub reclaim_blocked: bool,
    /// True when the measured profile remains above the budget; never reported as success below budget.
    pub pressure: bool,
}

/// Sanitized last maintenance failure, observable independently of timeline admission.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum DiskFault {
    /// Physical allocation could not be measured.
    Measurement,
    /// Media index or content maintenance failed.
    Media,
    /// History reclamation failed.
    History,
    /// Account lifecycle changed during maintenance.
    Session,
}
/// Last global budget observation; no watch value claims an unmeasured success.
#[derive(Clone, Copy, Debug)]
pub enum DiskStatus {
    /// No measurement has completed yet.
    Unmeasured,
    /// Actual physical allocation and remaining pressure.
    Measured(DiskReport),
    /// Maintenance failed; previous measurements must not be treated as current.
    Fault(DiskFault),
}

/// Physical measurement or reclamation failures, without paths or account data.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DiskError {
    /// A file could not be measured or changed during an unreadable scan.
    #[error("disk allocation measurement failed")]
    Measurement,
    /// Media reclamation failed.
    #[error(transparent)]
    Media(#[from] MediaError),
    /// History reclamation failed.
    #[error(transparent)]
    History(#[from] crate::HistoryError),
    /// Account/cache lifecycle failure.
    #[error(transparent)]
    Session(#[from] ClientError),
}

impl ClientHandle {
    /// Observes global disk pressure and independent maintenance faults.
    #[must_use]
    pub fn subscribe_disk(&self) -> tokio::sync::watch::Receiver<DiskStatus> {
        self.inner.disk_status.subscribe()
    }
    /// Measures physical allocation, including SQLite/WAL/SHM and shared media files.
    /// # Errors
    /// Returns revoked client, unreadable files or task failure. This never substitutes encoded-size estimates.
    pub async fn disk_usage(&self) -> Result<DiskReport, DiskError> {
        let client = self.clone();
        self.dispatch(async move { Ok(measure(&client).await) })
            .await?
    }

    /// Enforces the global physical ceiling: LRU media first, then oldest history in least recently opened channels.
    /// The report exposes remaining pressure from durable data, held readers or orphan grace; it never deletes intent.
    /// # Errors
    /// Returns measurement, media, history or lifecycle failures.
    pub async fn trim_disk(&self) -> Result<DiskReport, DiskError> {
        let client = self.clone();
        self.dispatch(async move { Ok(trim(&client).await) })
            .await?
    }
}

async fn measure(client: &ClientHandle) -> Result<DiskReport, DiskError> {
    let root = client.inner.media.root.clone();
    let budget = client.inner.config.disk.bytes;
    let lease = Arc::clone(&client.inner.registry.lease);
    tokio::task::spawn_blocking(move || {
        let _lease = lease;
        measurement::measure(&root, budget)
    })
    .await
    .map_err(|_| DiskError::Measurement)?
}

async fn trim(client: &ClientHandle) -> Result<DiskReport, DiskError> {
    let _serial = client.inner.disk_serial.lock().await;
    let initial = measure(client).await?;
    if !initial.pressure {
        return Ok(initial);
    }
    client.inner.media.collect(Duration::from_hours(24)).await?;
    let (candidates, caches) = victims::select(client).await?;
    // Reclaim already free pages and WAL before sacrificing any cached content.
    let mut reclaim_blocked = reclamation::existing(client, &caches).await?;
    let mut report = measure(client).await?;
    let mut media_evictions = 0;
    while report.pressure {
        let before_index = media_index_usage(client).await?;
        let Some(removed_bytes) = client.inner.media.evict_one().await? else {
            break;
        };
        media_evictions += 1;
        reclaim_blocked |= client
            .inner
            .media
            .writer
            .reclaim()
            .await
            .map_err(MediaError::from)?;
        let after_index = media_index_usage(client).await?;
        if media_evictions % 64 == 0 {
            report = measure(client).await?;
        } else {
            report.database_bytes = report
                .database_bytes
                .saturating_sub(before_index)
                .checked_add(after_index)
                .ok_or(DiskError::Measurement)?;
            report.media_bytes = report.media_bytes.saturating_sub(removed_bytes);
            report.allocated_bytes = report
                .database_bytes
                .checked_add(report.media_bytes)
                .and_then(|bytes| bytes.checked_add(report.outgoing_bytes))
                .and_then(|bytes| bytes.checked_add(report.profile_bytes))
                .ok_or(DiskError::Measurement)?;
            report.pressure = report.allocated_bytes > report.budget_bytes;
        }
    }
    // Concurrent writers can add allocation during a burst; the next tier uses a fresh scan.
    report = measure(client).await?;
    let history = reclamation::history(client, candidates, report).await?;
    reclaim_blocked |= history.blocked;
    reclaim_blocked |= reclamation::existing(client, &caches).await?;
    report = measure(client).await?;
    report.media_evictions = media_evictions;
    report.history_turns = history.turns;
    report.max_history_rows_per_turn = history.max_rows;
    report.reclaim_blocked = reclaim_blocked;
    report.reclaimed_bytes = initial
        .allocated_bytes
        .saturating_sub(report.allocated_bytes);
    Ok(report)
}

async fn media_index_usage(client: &ClientHandle) -> Result<u64, DiskError> {
    let path = client.inner.media.root.media_index();
    tokio::task::spawn_blocking(move || measurement::database(&path))
        .await
        .map_err(|_| DiskError::Measurement)?
}

pub(crate) fn start(client: ClientHandle) {
    let runtime = client.inner.runtime.clone();
    let token = client.inner.tasks.token();
    runtime.spawn(async move {
        let _token = token;
        loop {
            tokio::select! {
                biased;
                ()=client.inner.pool.root.cancelled()=>break,
                ()=tokio::time::sleep(client.inner.config.disk.maintenance_interval)=>{}
            }
            let result = trim(&client).await;
            let status = match result {
                Ok(report) => DiskStatus::Measured(report),
                Err(error) => DiskStatus::Fault(match error {
                    DiskError::Measurement => DiskFault::Measurement,
                    DiskError::Media(_) => DiskFault::Media,
                    DiskError::History(_) => DiskFault::History,
                    DiskError::Session(_) => DiskFault::Session,
                }),
            };
            client.inner.disk_status.send_replace(status);
        }
    });
}
