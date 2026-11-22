//! Background inbox sync, push wakeups, interactive sync preemption, and connection status bindings.
//! Disk budget measurement and maintenance live in `disk.rs`.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use papo_sdk::{
    BackgroundOutcome, BackgroundReport, ConnectionState, ConnectionStatus, InteractiveSyncGuard,
};
use tokio::time::Instant;

use crate::{
    client::FfiClient,
    errors::FfiClientError,
    ids::FfiAccountId,
    notification_records::{FfiNotificationDelivery, map_delivery},
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription, watch_stream},
};

/// Sanitized per-account background inbox sync outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiBackgroundOutcome {
    /// Bounded segment reconciled cleanly.
    Complete,
    /// More inbox pages remain for subsequent background work.
    More,
    /// Account transport, auth, or storage failed.
    Failed,
    /// Interactive app session preempted background sync.
    Preempted,
    /// Deadline or cancellation interrupted work.
    Interrupted,
}

impl From<BackgroundOutcome> for FfiBackgroundOutcome {
    fn from(value: BackgroundOutcome) -> Self {
        match value {
            BackgroundOutcome::Complete => Self::Complete,
            BackgroundOutcome::More => Self::More,
            BackgroundOutcome::Failed => Self::Failed,
            BackgroundOutcome::Preempted => Self::Preempted,
            // Interrupted and future variants map to Interrupted.
            _ => Self::Interrupted,
        }
    }
}

/// Per-account summary inside a background sync report.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiBackgroundAccountResult {
    /// Account processed.
    pub account: FfiAccountId,
    /// Account-isolated outcome.
    pub outcome: FfiBackgroundOutcome,
}

/// Result of `background_sync` or `push_wake`, carrying FULL-committed notification handoffs.
#[derive(Clone, uniffi::Record)]
pub struct FfiBackgroundReport {
    /// Independent outcome per processed account.
    pub accounts: Vec<FfiBackgroundAccountResult>,
    /// At-most-once FULL-committed platform notification handoffs.
    pub deliveries: Vec<FfiNotificationDelivery>,
}

impl From<BackgroundReport> for FfiBackgroundReport {
    fn from(report: BackgroundReport) -> Self {
        Self {
            accounts: report
                .accounts
                .into_iter()
                .map(|row| FfiBackgroundAccountResult {
                    account: FfiAccountId::from_sdk(row.account),
                    outcome: row.outcome.into(),
                })
                .collect(),
            deliveries: report.deliveries.into_iter().map(map_delivery).collect(),
        }
    }
}

/// Preemption guard holding interactive foreground priority while retained or unreleased.
#[derive(uniffi::Object)]
pub struct FfiInteractiveSyncGuard {
    /// Inner SDK interactive preemption guard (`None` once released).
    guard: Mutex<Option<InteractiveSyncGuard>>,
}

#[uniffi::export]
impl FfiInteractiveSyncGuard {
    /// Releases interactive sync preemption immediately without waiting for GC finalization.
    pub fn release(&self) {
        let _ = self
            .guard
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
}

/// Account transport connection state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiConnectionState {
    /// Establishing socket connection and running handshake/reconcile.
    Connecting,
    /// Live gateway online and reconciled for this generation.
    Online,
    /// Waiting in exponential backoff (`retry_in_ms` remaining from observation time).
    Backoff {
        /// Remaining delay until next connection attempt in milliseconds.
        retry_in_ms: u64,
    },
    /// Idle offline with no active retry loop.
    Offline,
}

/// Generation-tagged connection status for one account session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiConnectionStatus {
    /// Session connection epoch owning this state.
    pub generation: u64,
    /// Transport state.
    pub state: FfiConnectionState,
}

impl From<ConnectionStatus> for FfiConnectionStatus {
    fn from(status: ConnectionStatus) -> Self {
        let state = match status.state {
            ConnectionState::Connecting => FfiConnectionState::Connecting,
            ConnectionState::Online => FfiConnectionState::Online,
            ConnectionState::Backoff { retry_at } => {
                let remaining = retry_at.saturating_duration_since(Instant::now());
                let retry_in_ms = u64::try_from(remaining.as_millis()).unwrap_or(u64::MAX);
                FfiConnectionState::Backoff { retry_in_ms }
            }
            // Offline and unknown future states collapse to Offline.
            _ => FfiConnectionState::Offline,
        };
        Self {
            generation: status.generation.get(),
            state,
        }
    }
}

/// Listener for account transport connection updates.
#[uniffi::export(with_foreign)]
pub trait FfiConnectionListener: Send + Sync {
    /// Atomic baseline connection status.
    fn on_snapshot(&self, status: FfiConnectionStatus);
    /// Subsequent status updates until session closure or disposal.
    fn on_update(&self, status: FfiConnectionStatus);
    /// Terminal callback emitted when account session closes.
    fn on_closed(&self);
}

#[uniffi::export]
impl FfiClient {
    /// Preempts background sync while interactive UI ownership is active.
    #[must_use]
    pub fn interactive_sync(&self) -> Arc<FfiInteractiveSyncGuard> {
        Arc::new(FfiInteractiveSyncGuard {
            guard: Mutex::new(Some(self.handle.interactive_sync())),
        })
    }

    /// Runs deadline-bounded background inbox catch-up across all accounts.
    ///
    /// Dropping or canceling the Kotlin coroutine cancels underlying work automatically.
    ///
    /// # Errors
    /// Returns closed-client/executor failure; individual account failures stay inside the report.
    pub async fn background_sync(
        &self,
        deadline_ms: u64,
    ) -> Result<FfiBackgroundReport, FfiClientError> {
        let handle = self.handle.clone();
        let timeout = Duration::from_millis(deadline_ms.max(1));
        self.run_scoped(|| FfiClientError::Closed, async move {
            let deadline = Instant::now() + timeout;
            handle
                .background_sync(deadline)
                .await
                .map(Into::into)
                .map_err(FfiClientError::from)
        })
        .await
    }

    /// Handles a platform push wakeup strictly as an account-bound hint, fetching authoritative rows.
    ///
    /// # Errors
    /// Rejects invalid/unavailable account or closed client.
    pub async fn push_wake(
        &self,
        account: FfiAccountId,
        deadline_ms: u64,
    ) -> Result<FfiBackgroundReport, FfiClientError> {
        let parsed = account.into_sdk()?;
        let handle = self.handle.clone();
        let timeout = Duration::from_millis(deadline_ms.max(1));
        self.run_scoped(|| FfiClientError::Closed, async move {
            let deadline = Instant::now() + timeout;
            handle
                .push_wake(parsed, deadline)
                .await
                .map(Into::into)
                .map_err(FfiClientError::from)
        })
        .await
    }

    /// Reads the latest transport connection status for an active account session.
    ///
    /// # Errors
    /// Returns invalid argument, missing session, or closed client.
    pub fn connection_status(
        &self,
        account: FfiAccountId,
    ) -> Result<FfiConnectionStatus, FfiClientError> {
        let session = self.session_by_id(&account)?;
        Ok((*session.subscribe_connection().borrow()).into())
    }

    /// Requests a new connection generation on `account`'s session, canceling in-flight socket/REST work.
    ///
    /// # Errors
    /// Returns [`FfiClientError::SignedOut`] if signed out, or closure/argument failures.
    pub fn reconnect(&self, account: FfiAccountId) -> Result<(), FfiClientError> {
        let session = self.session_by_id(&account)?;
        session.reconnect().map_err(FfiClientError::from)
    }

    /// Subscribes to generation-stamped transport connection transitions for one account.
    ///
    /// # Errors
    /// Returns missing/revoked session or worker failures.
    pub fn subscribe_connection(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiConnectionListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let (baseline, stream) = watch_stream(session.subscribe_connection());
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
