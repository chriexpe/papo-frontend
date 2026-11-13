//! Deadline-bounded account-isolated notification catch-up; native schedulers and presentation live in clients.

use crate::{AccountId, ClientError, ClientHandle};
mod account;
use account::catch_up;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

/// Caller-owned cancellation capability for background work, separate from account lifetime.
pub use tokio_util::sync::CancellationToken as BackgroundCancellation;

pub(crate) struct BackgroundState {
    slots: Arc<tokio::sync::Semaphore>,
    interactive: Arc<AtomicUsize>,
    cancel: std::sync::Mutex<CancellationToken>,
}
impl Default for BackgroundState {
    fn default() -> Self {
        Self {
            slots: Arc::new(tokio::sync::Semaphore::new(4)),
            interactive: Arc::default(),
            cancel: std::sync::Mutex::new(CancellationToken::new()),
        }
    }
}
/// Interactive ownership preempts background work until all guards are dropped.
pub struct InteractiveSyncGuard(Arc<AtomicUsize>);
impl Drop for InteractiveSyncGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
/// Sanitized per-account outcome; a failed server never aborts healthy accounts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum BackgroundOutcome {
    /// Current bounded inbox segment reconciled.
    Complete,
    /// More pages remain for the next invocation.
    More,
    /// Authentication or transport/storage work failed.
    Failed,
    /// Interactive work replaced this run.
    Preempted,
    /// Deadline or caller cancellation interrupted the run.
    Interrupted,
}
/// One account's result, without server addresses or error text.
#[derive(Clone)]
pub struct BackgroundAccountResult {
    /// Source account.
    pub account: AccountId,
    /// Independent outcome.
    pub outcome: BackgroundOutcome,
}
/// Claimed handoffs and account progress. Every delivery has already committed to FULL storage.
#[derive(Clone, Default)]
pub struct BackgroundReport {
    /// Independent per-account statuses.
    pub accounts: Vec<BackgroundAccountResult>,
    /// Platform handoffs; validate each target before presentation.
    pub deliveries: Vec<crate::NotificationDelivery>,
}
impl ClientHandle {
    /// Preempts outstanding background work and prevents new admission while interactive ownership exists.
    #[must_use]
    pub fn interactive_sync(&self) -> InteractiveSyncGuard {
        let state = &self.inner.background;
        let mut cancel = state
            .cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.interactive.fetch_add(1, Ordering::AcqRel);
        cancel.cancel();
        *cancel = CancellationToken::new();
        InteractiveSyncGuard(Arc::clone(&state.interactive))
    }
    /// Reconciles bounded account inboxes without native UI. Dropping the future cancels its work.
    /// # Errors
    /// Returns closed client/executor failures. Account failures are isolated in the report.
    pub async fn background_sync(
        &self,
        deadline: Instant,
    ) -> Result<BackgroundReport, ClientError> {
        self.background_sync_cancelable(deadline, CancellationToken::new())
            .await
    }
    /// Like `background_sync`, with explicit cancellation supplied by a platform scheduler.
    /// # Errors
    /// Returns closed client/executor failures; no stale account result is handed off.
    pub async fn background_sync_cancelable(
        &self,
        deadline: Instant,
        cancel: CancellationToken,
    ) -> Result<BackgroundReport, ClientError> {
        let this = self.clone();
        // Cancellation-on-drop also covers dispatch's independently scheduled future.
        let dropped = CancelOnDrop(CancellationToken::new());
        let drop_token = dropped.0.clone();
        self.dispatch(async move { run(this, deadline, cancel, drop_token, None).await })
            .await
    }
}
impl ClientHandle {
    /// Treats a push payload solely as an account-bound wake hint, then fetches authoritative rows.
    /// # Errors
    /// Rejects an unavailable account or closed executor. Never trusts push preview/notification IDs.
    pub async fn push_wake(
        &self,
        account: AccountId,
        deadline: Instant,
    ) -> Result<BackgroundReport, ClientError> {
        self.session(account)?;
        let this = self.clone();
        let dropped = CancelOnDrop(CancellationToken::new());
        let token = dropped.0.clone();
        self.dispatch(async move {
            run(
                this,
                deadline,
                CancellationToken::new(),
                token,
                Some(account),
            )
            .await
        })
        .await
    }
}

struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
async fn run(
    client: ClientHandle,
    deadline: Instant,
    cancel: CancellationToken,
    dropped: CancellationToken,
    account: Option<AccountId>,
) -> Result<BackgroundReport, ClientError> {
    let preemption = {
        let token = client
            .inner
            .background
            .cancel
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        token.child_token()
    };
    let accounts = client
        .inner
        .registry_rx
        .borrow()
        .accounts
        .iter()
        .map(|a| a.id)
        .filter(|id| account.is_none_or(|account| account == *id))
        .collect::<Vec<_>>();
    let mut report = BackgroundReport::default();
    if client.inner.background.interactive.load(Ordering::Acquire) > 0 {
        report.accounts = accounts
            .into_iter()
            .map(|account| BackgroundAccountResult {
                account,
                outcome: BackgroundOutcome::Preempted,
            })
            .collect();
        return Ok(report);
    }
    let mut jobs = tokio::task::JoinSet::new();
    let slots = Arc::clone(&client.inner.background.slots);
    for account in accounts {
        let session = client.session(account);
        let cancel = cancel.clone();
        let dropped = dropped.clone();
        let preemption = preemption.clone();
        let slots = Arc::clone(&slots);
        jobs.spawn(async move {
            let (outcome, deliveries) =
                account_job(session, slots, preemption, cancel, dropped, deadline).await;
            (BackgroundAccountResult { account, outcome }, deliveries)
        });
    }
    while let Some(result) = jobs.join_next().await {
        let (account, deliveries) = result.map_err(|_| ClientError::Executor)?;
        report.accounts.push(account);
        for delivery in deliveries {
            if let Ok(session) = client.session(delivery.target.account)
                && session
                    .inner
                    .epochs
                    .validate(delivery.target.generation)
                    .is_ok()
                && !preemption.is_cancelled()
            {
                report.deliveries.push(delivery);
            }
        }
    }
    report.deliveries.retain(|delivery| {
        !preemption.is_cancelled()
            && !cancel.is_cancelled()
            && !dropped.is_cancelled()
            && client
                .session(delivery.target.account)
                .is_ok_and(|session| {
                    session
                        .inner
                        .epochs
                        .validate(delivery.target.generation)
                        .is_ok()
                })
    });
    Ok(report)
}

async fn account_job(
    session: Result<crate::Session, ClientError>,
    slots: Arc<tokio::sync::Semaphore>,
    preemption: CancellationToken,
    cancel: CancellationToken,
    dropped: CancellationToken,
    deadline: Instant,
) -> (BackgroundOutcome, Vec<crate::NotificationDelivery>) {
    let mut deliveries = Vec::new();
    let work = async {
        let _permit = slots
            .acquire()
            .await
            .map_err(|_| crate::NotificationError::from(ClientError::Closed))?;
        let session = session.map_err(crate::NotificationError::from)?;
        catch_up(&session, &mut deliveries).await
    };
    let result = tokio::select! {
        biased;
        () = preemption.cancelled() => BackgroundOutcome::Preempted,
        () = cancel.cancelled() => BackgroundOutcome::Interrupted,
        () = dropped.cancelled() => BackgroundOutcome::Interrupted,
        () = tokio::time::sleep_until(deadline) => BackgroundOutcome::Interrupted,
        result = work => match result {
            Ok(result) => result,
            Err(_) => BackgroundOutcome::Failed,
        },
    };
    if matches!(
        result,
        BackgroundOutcome::Interrupted | BackgroundOutcome::Preempted
    ) {
        deliveries.clear();
    }
    (result, deliveries)
}
