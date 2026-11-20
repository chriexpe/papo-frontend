//! Client-wide single-call lease across accounts, including `ReleasePending` device lock retention.

use crate::{AccountId, Uuid};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::Notify;

use super::model::{CallAttemptId, CallError};

/// Snapshot of who currently owns the client-wide call lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ActiveCallLease {
    /// Account owning the call.
    pub account: AccountId,
    /// Attempt owning the call.
    pub attempt: CallAttemptId,
    /// Voice channel of the call.
    pub channel: Uuid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeaseState {
    Free,
    Active(ActiveCallLease),
    /// Previous call ended and is draining its media lease; `next` may already be queued for same-account switch.
    Draining {
        draining: CallAttemptId,
        active: Option<ActiveCallLease>,
    },
    /// Previous media teardown missed its deadline; hardware lease stays held until `Drained` arrives.
    ReleasePending {
        attempt: CallAttemptId,
    },
}

/// Coordinates the client-wide single-call lease and monotonic attempt allocation.
pub(crate) struct ClientCallLease {
    next_attempt: AtomicU64,
    state: Mutex<LeaseState>,
    drain_notify: Notify,
}

impl Default for ClientCallLease {
    fn default() -> Self {
        Self {
            next_attempt: AtomicU64::new(1),
            state: Mutex::new(LeaseState::Free),
            drain_notify: Notify::new(),
        }
    }
}

impl ClientCallLease {
    /// Allocates a fresh, never-reused `CallAttemptId`.
    pub(crate) fn allocate_attempt(&self) -> CallAttemptId {
        CallAttemptId::from_raw(self.next_attempt.fetch_add(1, Ordering::Relaxed))
    }

    /// Returns the currently active `(account, attempt, channel)` if any.
    pub(crate) fn active(&self) -> Option<ActiveCallLease> {
        let guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *guard {
            LeaseState::Active(lease) => Some(lease),
            LeaseState::Draining { active, .. } => active,
            LeaseState::Free | LeaseState::ReleasePending { .. } => None,
        }
    }

    /// Returns true if a previous attempt's media release timed out and is still pending.
    pub(crate) fn is_release_pending(&self) -> bool {
        let guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        matches!(*guard, LeaseState::ReleasePending { .. })
    }

    /// Attempts to claim the client lease for `account` joining `channel`.
    ///
    /// Same-account re-join replaces the active attempt immediately while preserving any
    /// in-flight media drain handle so the new attempt waits for devices before `start()`.
    pub(crate) fn acquire(
        &self,
        account: AccountId,
        channel: Uuid,
        previous_needs_drain: Option<CallAttemptId>,
    ) -> Result<(CallAttemptId, Option<CallAttemptId>), CallError> {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *guard {
            LeaseState::ReleasePending { .. } => Err(CallError::ReleasePending),
            LeaseState::Active(current) if current.account != account => Err(CallError::Busy),
            LeaseState::Draining {
                active: Some(current),
                ..
            } if current.account != account => Err(CallError::Busy),
            LeaseState::Free => {
                let attempt = self.allocate_attempt();
                let next = ActiveCallLease {
                    account,
                    attempt,
                    channel,
                };
                *guard = if let Some(draining) = previous_needs_drain {
                    LeaseState::Draining {
                        draining,
                        active: Some(next),
                    }
                } else {
                    LeaseState::Active(next)
                };
                Ok((attempt, previous_needs_drain))
            }
            LeaseState::Active(current) => {
                let attempt = self.allocate_attempt();
                let next = ActiveCallLease {
                    account,
                    attempt,
                    channel,
                };
                let draining = previous_needs_drain.or(Some(current.attempt));
                *guard = if let Some(draining) = draining {
                    LeaseState::Draining {
                        draining,
                        active: Some(next),
                    }
                } else {
                    LeaseState::Active(next)
                };
                Ok((attempt, draining))
            }
            LeaseState::Draining { draining, .. } => {
                let attempt = self.allocate_attempt();
                let next = ActiveCallLease {
                    account,
                    attempt,
                    channel,
                };
                let wait_for = previous_needs_drain.unwrap_or(draining);
                *guard = LeaseState::Draining {
                    draining: wait_for,
                    active: Some(next),
                };
                Ok((attempt, Some(wait_for)))
            }
        }
    }

    /// Marks an active attempt as ended, transitioning to `Draining` (if media was started) or `Free`.
    pub(crate) fn begin_end(&self, attempt: CallAttemptId, needs_drain: bool) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *guard {
            LeaseState::Active(current) if current.attempt == attempt => {
                *guard = if needs_drain {
                    LeaseState::Draining {
                        draining: attempt,
                        active: None,
                    }
                } else {
                    LeaseState::Free
                };
                if !needs_drain {
                    self.drain_notify.notify_waiters();
                }
            }
            LeaseState::Draining {
                draining,
                active: Some(current),
            } if current.attempt == attempt => {
                *guard = LeaseState::Draining {
                    draining: if needs_drain { attempt } else { draining },
                    active: None,
                };
            }
            _ => {}
        }
    }

    /// Marks `attempt`'s media teardown as completely drained.
    pub(crate) fn mark_drained(&self, attempt: CallAttemptId) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let changed = match *guard {
            LeaseState::Draining { draining, active } if draining == attempt => {
                *guard = match active {
                    Some(lease) => LeaseState::Active(lease),
                    None => LeaseState::Free,
                };
                true
            }
            LeaseState::ReleasePending { attempt: stuck } if stuck == attempt => {
                *guard = LeaseState::Free;
                true
            }
            LeaseState::Active(current) if current.attempt == attempt => {
                *guard = LeaseState::Free;
                true
            }
            _ => false,
        };
        drop(guard);
        if changed {
            self.drain_notify.notify_waiters();
        }
    }

    /// Records that `attempt` missed its drain deadline; keeps hardware lease locked until drained.
    pub(crate) fn mark_release_pending(&self, attempt: CallAttemptId) {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match *guard {
            LeaseState::Draining { draining, .. } if draining == attempt => {
                *guard = LeaseState::ReleasePending { attempt };
            }
            LeaseState::Active(current) if current.attempt == attempt => {
                *guard = LeaseState::ReleasePending { attempt };
            }
            _ => {}
        }
        drop(guard);
        self.drain_notify.notify_waiters();
    }

    /// Waits until `attempt` is no longer draining (either drained or transitioned to `ReleasePending`).
    pub(crate) async fn wait_drained(&self, attempt: CallAttemptId) -> Result<(), CallError> {
        loop {
            let notified = self.drain_notify.notified();
            {
                let guard = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                match *guard {
                    LeaseState::ReleasePending { attempt: stuck } if stuck == attempt => {
                        return Err(CallError::ReleasePending);
                    }
                    LeaseState::Draining { draining, .. } if draining == attempt => {}
                    LeaseState::Active(current) if current.attempt == attempt => {}
                    _ => return Ok(()),
                }
            }
            notified.await;
        }
    }
}
