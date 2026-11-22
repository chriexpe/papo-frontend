//! Disk budget reporting, explicit reclamation, and disk pressure monitoring across FFI.
//! Background sync and gateway connection status live in `sync_connection.rs`.

use std::sync::Arc;

use papo_sdk::{DiskFault, DiskReport, DiskStatus};

use crate::{
    client::FfiClient,
    errors::FfiClientError,
    media_disk_errors::FfiDiskError,
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription, watch_stream},
};

/// Physical filesystem usage measurement across SDK databases, sidecars, media, and outgoing blobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiDiskReport {
    /// Configured physical byte limit.
    pub budget_bytes: u64,
    /// Physical bytes allocated on disk across all SDK roots.
    pub allocated_bytes: u64,
    /// SQLite database files plus WAL/SHM/journal sidecars.
    pub database_bytes: u64,
    /// Shared content-addressed media files including orphans.
    pub media_bytes: u64,
    /// Protected durable outgoing staging blobs.
    pub outgoing_bytes: u64,
    /// Profile lock and marker bytes.
    pub profile_bytes: u64,
    /// Physical bytes freed in this trim burst.
    pub reclaimed_bytes: u64,
    /// LRU media files removed.
    pub media_evictions: u64,
    /// History retention transactions committed.
    pub history_turns: u64,
    /// Largest history mutation row count in a single turn.
    pub max_history_rows_per_turn: u64,
    /// True if an active reader blocked SQLite checkpoint truncation.
    pub reclaim_blocked: bool,
    /// True if physical allocation still exceeds `budget_bytes`.
    pub pressure: bool,
}

impl From<DiskReport> for FfiDiskReport {
    fn from(value: DiskReport) -> Self {
        Self {
            budget_bytes: value.budget_bytes,
            allocated_bytes: value.allocated_bytes,
            database_bytes: value.database_bytes,
            media_bytes: value.media_bytes,
            outgoing_bytes: value.outgoing_bytes,
            profile_bytes: value.profile_bytes,
            reclaimed_bytes: value.reclaimed_bytes,
            media_evictions: value.media_evictions,
            history_turns: value.history_turns,
            max_history_rows_per_turn: u64::try_from(value.max_history_rows_per_turn)
                .unwrap_or(u64::MAX),
            reclaim_blocked: value.reclaim_blocked,
            pressure: value.pressure,
        }
    }
}

/// Sanitized background disk maintenance fault kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiDiskFault {
    /// Physical measurement failed.
    Measurement,
    /// Media maintenance failed.
    Media,
    /// Post-commit resource cache maintenance failed.
    ResourceMaintenance,
    /// History reclamation failed.
    History,
    /// Account lifecycle interrupted maintenance.
    Session,
}

impl From<DiskFault> for FfiDiskFault {
    fn from(value: DiskFault) -> Self {
        match value {
            DiskFault::Media => Self::Media,
            DiskFault::ResourceMaintenance => Self::ResourceMaintenance,
            DiskFault::History => Self::History,
            DiskFault::Session => Self::Session,
            // Measurement and future faults map to Measurement.
            _ => Self::Measurement,
        }
    }
}

/// Latest observable disk budget status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiDiskStatus {
    /// No measurement completed yet.
    Unmeasured,
    /// Exact physical measurement and pressure state.
    Measured {
        /// Latest disk report.
        report: FfiDiskReport,
    },
    /// Latest maintenance step failed; earlier measurements are stale.
    Fault {
        /// Sanitized failure kind.
        fault: FfiDiskFault,
    },
}

impl From<DiskStatus> for FfiDiskStatus {
    fn from(value: DiskStatus) -> Self {
        match value {
            DiskStatus::Unmeasured => Self::Unmeasured,
            DiskStatus::Measured(report) => Self::Measured {
                report: report.into(),
            },
            DiskStatus::Fault(fault) => Self::Fault {
                fault: fault.into(),
            },
        }
    }
}

/// Listener for profile disk budget updates.
#[uniffi::export(with_foreign)]
pub trait FfiDiskStatusListener: Send + Sync {
    /// Initial atomic status captured on subscription.
    fn on_snapshot(&self, status: FfiDiskStatus);
    /// Subsequent status changes until disposal or shutdown.
    fn on_update(&self, status: FfiDiskStatus);
    /// Terminal notification after watch ends.
    fn on_closed(&self);
}

#[uniffi::export]
impl FfiClient {
    /// Measures allocated disk bytes across SQLite, media, outgoing, and profile files.
    ///
    /// # Errors
    /// Returns filesystem measurement or client closure errors.
    pub async fn disk_usage(&self) -> Result<FfiDiskReport, FfiDiskError> {
        let handle = self.handle.clone();
        self.run_scoped(
            || FfiDiskError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                handle
                    .disk_usage()
                    .await
                    .map(Into::into)
                    .map_err(Into::into)
            },
        )
        .await
    }

    /// Runs explicit cache reclamation toward configured budget, returning updated allocation report.
    ///
    /// # Errors
    /// Returns measurement, media, history, or client closure errors.
    pub async fn trim_disk(&self) -> Result<FfiDiskReport, FfiDiskError> {
        let handle = self.handle.clone();
        self.run_scoped(
            || FfiDiskError::Session {
                error: FfiClientError::Closed,
            },
            async move { handle.trim_disk().await.map(Into::into).map_err(Into::into) },
        )
        .await
    }

    /// Subscribes to global disk budget status updates without polling.
    ///
    /// # Errors
    /// Returns worker failure or closed client if already shut down.
    pub fn subscribe_disk(
        &self,
        listener: Arc<dyn FfiDiskStatusListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let (baseline, stream) = watch_stream(self.handle.subscribe_disk());
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.runtime,
            &self.tasks,
            &self.shutdown,
            self.callbacks.clone(),
            baseline,
            stream,
            move |status| {
                l_snap.on_snapshot(status.into());
                true
            },
            move |status| {
                l_upd.on_update(status.into());
                true
            },
            move || l_end.on_closed(),
        )
    }
}
