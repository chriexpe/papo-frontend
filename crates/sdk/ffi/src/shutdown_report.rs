//! Storage worker and foreign callback shutdown diagnostic records for FFI.
//! Process client lifecycle management lives in `client.rs`.

use papo_sdk::storage_worker::{
    DatabaseFailure, ShutdownReport, UnpersistedWrite, WorkerError, WriteDisposition,
};

use crate::subscription::FfiCallbackDrainStatus;

/// Disposition of an unpersisted SQLite write reported by worker shutdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiWriteDisposition {
    /// Queued command never began execution.
    NotStarted,
    /// Worker/callback ran; physical commit state at deadline is unknown.
    OutcomeUnknown,
    /// Transaction returned an explicit error.
    Failed,
}

impl From<WriteDisposition> for FfiWriteDisposition {
    fn from(value: WriteDisposition) -> Self {
        match value {
            WriteDisposition::NotStarted => Self::NotStarted,
            WriteDisposition::Failed => Self::Failed,
            // OutcomeUnknown and future dispositions conservatively preserve OutcomeUnknown.
            _ => Self::OutcomeUnknown,
        }
    }
}

/// One unacknowledged database write observed at shutdown deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiUnpersistedWrite {
    /// Worker-local database ID.
    pub database_id: u64,
    /// Worker-local accepted command ID.
    pub command_id: u64,
    /// Whether a late physical commit remains possible.
    pub disposition: FfiWriteDisposition,
}

impl From<&UnpersistedWrite> for FfiUnpersistedWrite {
    fn from(value: &UnpersistedWrite) -> Self {
        Self {
            database_id: value.database_id,
            command_id: value.command_id,
            disposition: value.disposition.into(),
        }
    }
}

/// Sanitized storage failure recorded on one database during worker shutdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiDatabaseFailure {
    /// Worker-local database ID.
    pub database_id: u64,
    /// Whether SQLite returned an extended error code.
    pub sqlite_code: Option<i32>,
    /// True if shutdown deadline interrupted waiting.
    pub deadline_reached: bool,
}

impl From<&DatabaseFailure> for FfiDatabaseFailure {
    fn from(value: &DatabaseFailure) -> Self {
        let (sqlite_code, deadline_reached) = match value.error {
            WorkerError::Sqlite { code } => (code, false),
            WorkerError::ShutdownDeadline => (None, true),
            _ => (None, false),
        };
        Self {
            database_id: value.database_id,
            sqlite_code,
            deadline_reached,
        }
    }
}

/// Drain summary for one storage writer thread (`state` or `shared`).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiWorkerShutdownReport {
    /// True when connections checkpointed and worker thread joined.
    pub finished: bool,
    /// True when drain waited until its timeout.
    pub deadline_reached: bool,
    /// Commands not confirmed committed before exit.
    pub unpersisted: Vec<FfiUnpersistedWrite>,
    /// Isolated per-database failures observed during shutdown.
    pub failures: Vec<FfiDatabaseFailure>,
}

impl From<&ShutdownReport> for FfiWorkerShutdownReport {
    fn from(report: &ShutdownReport) -> Self {
        Self {
            finished: report.finished,
            deadline_reached: report.deadline_reached,
            unpersisted: report.unpersisted.iter().map(Into::into).collect(),
            failures: report.failures.iter().map(Into::into).collect(),
        }
    }
}

/// Complete client shutdown outcome including honest foreign callback drain status.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiShutdownReport {
    /// False if a blocking SQLite read pool worker was still running at drain deadline.
    pub reads_drained: bool,
    /// Durable state SQLite worker drain status.
    pub state: FfiWorkerShutdownReport,
    /// Shared rebuildable cache SQLite worker drain status.
    pub shared: FfiWorkerShutdownReport,
    /// Bounded foreign subscription callback drain status at exit.
    pub callback_drain: FfiCallbackDrainStatus,
}
