//! Handle-based public API (`Calls`, `CallHandle`, `HostLease`) over `SessionCalls`.
//! Every `CallHandle` and `HostLease` belongs to one attempt and becomes inert when it ends.

use crate::{AccountId, ClientHandle, Session, Uuid, observable::StateCell};
use std::sync::Arc;
use tokio::sync::watch;

use super::{
    controls,
    coordinator::SessionCalls,
    media_port::VideoDestination,
    model::{
        CallAttemptId, CallEndReason, CallError, CallSnapshot, CameraFacing, HostLeaseKind,
        RoomsSnapshot, VideoKind, VideoTarget,
    },
    startup::SessionIceFetcher,
};

/// Account-scoped call entry point returned by `Session::calls()`.
#[derive(Clone)]
pub struct Calls {
    session: Session,
}

impl Calls {
    pub(crate) fn new(session: Session) -> Self {
        Self { session }
    }

    /// Subscribes to live voice rooms for this account, filtered by admitted `connect_voice`.
    ///
    /// Accounts without a media engine factory can still watch voice rooms.
    #[must_use]
    pub fn subscribe_rooms(&self) -> watch::Receiver<RoomsSnapshot> {
        self.session.inner.calls.rooms_cell.subscribe()
    }

    /// Subscribes to this session's active call snapshot (`None` when idle or ended).
    #[must_use]
    pub fn subscribe_active(&self) -> watch::Receiver<Option<CallSnapshot>> {
        self.session.inner.calls.active_cell.subscribe()
    }

    /// Joins `channel` on this session's socket, returning a `CallHandle` immediately.
    ///
    /// Leaving a previous call on this same account and joining `channel` run back-to-back on
    /// the same socket with no barrier timer or socket rotation. Joining while another account
    /// holds the client-wide lease returns `CallError::Busy`; use `ClientHandle::switch_call`.
    ///
    /// # Errors
    /// Returns typed precondition or lease failures (`Busy`, `Offline`, `Forbidden`,
    /// `PermissionUnknown`, `NotVoiceChannel`, `MediaUnavailable`, `ReleasePending`, `Closed`).
    pub fn join(&self, channel: Uuid) -> Result<CallHandle, CallError> {
        let media_factory = self.session.client.inner.config.media_engine.clone();
        let ice_fetcher = Arc::new(SessionIceFetcher {
            client: self.session.client.clone(),
            session: Arc::clone(&self.session.inner),
        });
        let (attempt, snapshot_cell) = self.session.inner.calls.join(
            channel,
            media_factory,
            ice_fetcher,
            &self.session.client.inner.runtime,
            &self.session.inner.root,
        )?;
        Ok(CallHandle {
            account: self.session.account(),
            attempt,
            calls: Arc::clone(&self.session.inner.calls),
            snapshot_cell,
            runtime: self.session.client.inner.runtime.clone(),
        })
    }

    /// Reattaches to this session's currently active call attempt after navigation or view rebuild.
    #[must_use]
    pub fn active(&self) -> Option<CallHandle> {
        let (attempt, snapshot_cell) = self.session.inner.calls.active_attempt_handle_parts()?;
        Some(CallHandle {
            account: self.session.account(),
            attempt,
            calls: Arc::clone(&self.session.inner.calls),
            snapshot_cell,
            runtime: self.session.client.inner.runtime.clone(),
        })
    }
}

/// Handle to a single call attempt; goes permanently inert when that attempt ends.
#[derive(Clone)]
pub struct CallHandle {
    account: AccountId,
    attempt: CallAttemptId,
    calls: Arc<SessionCalls>,
    snapshot_cell: Arc<StateCell<CallSnapshot>>,
    runtime: tokio::runtime::Handle,
}

impl CallHandle {
    #[cfg(test)]
    pub(crate) fn from_parts(
        account: AccountId,
        attempt: CallAttemptId,
        calls: Arc<SessionCalls>,
        snapshot_cell: Arc<StateCell<CallSnapshot>>,
        runtime: tokio::runtime::Handle,
    ) -> Self {
        Self {
            account,
            attempt,
            calls,
            snapshot_cell,
            runtime,
        }
    }

    /// Account owning this attempt.
    #[must_use]
    pub const fn account(&self) -> AccountId {
        self.account
    }

    /// Unique identifier for this call attempt.
    #[must_use]
    pub const fn attempt(&self) -> CallAttemptId {
        self.attempt
    }

    /// Current state snapshot for this attempt.
    #[must_use]
    pub fn snapshot(&self) -> CallSnapshot {
        self.snapshot_cell.borrow().clone()
    }

    /// Subscribes to state updates for this attempt; closes once `CallPhase::Ended` is published.
    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<CallSnapshot> {
        self.snapshot_cell.subscribe()
    }

    /// Leaves this attempt; safe and idempotent if the attempt has already ended.
    pub fn leave(&self) {
        controls::leave_attempt(
            &self.calls,
            self.attempt,
            CallEndReason::LocalLeave,
            &self.runtime,
        );
    }

    /// Sets local microphone mute intent.
    ///
    /// # Errors
    /// Returns `CallError::Ended` if this attempt has already ended.
    pub fn set_muted(&self, muted: bool) -> Result<(), CallError> {
        controls::set_muted(&self.calls, self.attempt, muted, &self.runtime)
    }

    /// Sets local camera publication intent (`voice_camera` is ordered ahead of the offer).
    ///
    /// # Errors
    /// Returns `CallError::Ended` if this attempt has already ended.
    pub fn set_camera(&self, on: bool) -> Result<(), CallError> {
        controls::set_camera(&self.calls, self.attempt, on, &self.runtime)
    }

    /// Sets local camera facing direction.
    ///
    /// # Errors
    /// Returns `CallError::Ended` if this attempt has already ended.
    pub fn set_camera_facing(&self, facing: CameraFacing) -> Result<(), CallError> {
        controls::set_camera_facing(&self.calls, self.attempt, facing, &self.runtime)
    }

    /// Sets independent local receiver playback gain (`0..=200`) and mute for `user`.
    ///
    /// # Errors
    /// Returns `CallError::Ended` if this attempt has already ended.
    pub fn set_playback(
        &self,
        user: Uuid,
        gain_percent: u16,
        muted: bool,
    ) -> Result<(), CallError> {
        controls::set_playback(
            &self.calls,
            self.attempt,
            user,
            gain_percent,
            muted,
            &self.runtime,
        )
    }

    /// Updates the ordered set of remote video streams currently wanted by the UI.
    ///
    /// # Errors
    /// Returns `CallError::Ended` if this attempt has already ended.
    pub fn set_video_interest(&self, targets: Vec<VideoTarget>) -> Result<(), CallError> {
        controls::set_video_interest(&self.calls, self.attempt, targets, &self.runtime)
    }

    /// Attaches a host video destination to `(participant, kind)`.
    ///
    /// # Errors
    /// Returns `CallError::Ended` if this attempt has already ended.
    pub fn bind_video(
        &self,
        participant: Uuid,
        kind: VideoKind,
        destination: Arc<dyn VideoDestination>,
    ) -> Result<(), CallError> {
        controls::bind_video(
            &self.calls,
            self.attempt,
            participant,
            kind,
            destination,
            &self.runtime,
        )
    }

    /// Detaches and revokes any host video destination bound to `(participant, kind)`.
    ///
    /// # Errors
    /// Returns `CallError::Ended` if this attempt has already ended.
    pub fn unbind_video(&self, participant: Uuid, kind: VideoKind) -> Result<(), CallError> {
        controls::unbind_video(&self.calls, self.attempt, participant, kind, &self.runtime)
    }

    /// Platform host callback: OS audio focus was lost (mutes this attempt only).
    ///
    /// # Errors
    /// Returns `CallError::Ended` if this attempt has already ended.
    pub fn on_audio_focus_lost(&self) -> Result<(), CallError> {
        controls::on_audio_focus_lost(&self.calls, self.attempt, &self.runtime)
    }

    /// Platform host callback: OS audio focus was regained (never unmutes automatically).
    ///
    /// # Errors
    /// Returns `CallError::Ended` if this attempt has already ended.
    pub fn on_audio_focus_regained(&self) -> Result<(), CallError> {
        controls::on_audio_focus_regained(&self.calls, self.attempt, &self.runtime)
    }

    /// Platform host callback: OS stopped the call service (ends this attempt only).
    ///
    /// # Errors
    /// Returns `CallError::Ended` if this attempt has already ended.
    pub fn on_host_terminated(&self) -> Result<(), CallError> {
        if !self.snapshot().phase.is_active() {
            return Err(CallError::Ended);
        }
        controls::leave_attempt(
            &self.calls,
            self.attempt,
            CallEndReason::HostTerminated,
            &self.runtime,
        );
        Ok(())
    }

    /// Registers a compare-on-release host lease (e.g. audio focus or foreground service) for this attempt.
    ///
    /// # Errors
    /// Returns `CallError::Ended` if this attempt has already ended.
    pub fn acquire_host_lease(&self, kind: HostLeaseKind) -> Result<HostLease, CallError> {
        let lease_id = controls::acquire_host_lease(&self.calls, self.attempt, kind)?;
        Ok(HostLease {
            attempt: self.attempt,
            kind,
            lease_id,
            calls: Arc::clone(&self.calls),
        })
    }
}

/// Compare-on-release host lease token bound to one call attempt.
///
/// Releasing a stale lease from an earlier attempt never touches a newer call.
pub struct HostLease {
    attempt: CallAttemptId,
    kind: HostLeaseKind,
    lease_id: u64,
    calls: Arc<SessionCalls>,
}

impl HostLease {
    /// Attempt this host lease belongs to.
    #[must_use]
    pub const fn attempt(&self) -> CallAttemptId {
        self.attempt
    }

    /// Lease category.
    #[must_use]
    pub const fn kind(&self) -> HostLeaseKind {
        self.kind
    }

    /// Releases this host lease if and only if it still belongs to the active attempt.
    /// Returns `true` if released, or `false` if this lease was already stale/superseded.
    #[must_use]
    pub fn release(self) -> bool {
        controls::release_host_lease(&self.calls, self.attempt, self.kind, self.lease_id)
    }
}

impl Session {
    /// Returns this account's call control and room-observation entry point.
    #[must_use]
    pub fn calls(&self) -> Calls {
        Calls::new(self.clone())
    }
}

impl ClientHandle {
    /// Returns the currently active call across all accounts, if any.
    #[must_use]
    pub fn active_call(&self) -> Option<(AccountId, CallHandle)> {
        let lease = self.inner.call_lease.active()?;
        let session = self.session(lease.account).ok()?;
        let handle = session.calls().active()?;
        Some((lease.account, handle))
    }

    /// Transfers the client-wide call lease to `(account, channel)`.
    ///
    /// Validates target preconditions first, leaves any active call on another account, waits
    /// for its media lease to drain within `CallConfig::drain_timeout`, and joins on the target
    /// session's socket. Switching the viewed account in UI does not call this.
    ///
    /// # Errors
    /// Returns typed precondition, drain (`ReleasePending`) or lifecycle errors.
    pub async fn switch_call(
        &self,
        account: AccountId,
        channel: Uuid,
    ) -> Result<CallHandle, CallError> {
        if self.is_closed() {
            return Err(CallError::Closed);
        }
        if self.inner.config.media_engine.is_none() {
            return Err(CallError::MediaUnavailable);
        }
        if self.inner.call_lease.is_release_pending() {
            return Err(CallError::ReleasePending);
        }
        let target = self.session(account).map_err(|error| match error {
            crate::ClientError::Closed => CallError::Closed,
            _ => CallError::AccountNotFound,
        })?;
        // Verify target session is online and authorized before tearing down an existing call.
        {
            let status = *target.subscribe_connection().borrow();
            if status.state != crate::ConnectionState::Online
                || status.generation != target.generation()
            {
                return Err(CallError::Offline);
            }
            let resources = target.subscribe_resources().borrow().clone();
            if resources.generation != status.generation {
                return Err(CallError::PermissionUnknown);
            }
            match crate::permissions::voice_access(&resources, account.user(), channel) {
                crate::permissions::VoiceAccess::Allowed => {}
                crate::permissions::VoiceAccess::NotVoiceChannel => {
                    return Err(CallError::NotVoiceChannel);
                }
                crate::permissions::VoiceAccess::Denied => return Err(CallError::Forbidden),
                crate::permissions::VoiceAccess::Unknown => {
                    return Err(CallError::PermissionUnknown);
                }
            }
        }

        if let Some(current) = self.inner.call_lease.active()
            && current.account != account
            && let Ok(old_session) = self.session(current.account)
        {
            let draining = old_session
                .inner
                .calls
                .end_active(CallEndReason::Switched, Some(&self.inner.runtime));
            if let Some(prev_attempt) = draining {
                let waited = tokio::time::timeout(
                    self.inner.config.call.drain_timeout,
                    self.inner.call_lease.wait_drained(prev_attempt),
                )
                .await;
                match waited {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => return Err(error),
                    Err(_) => {
                        self.inner.call_lease.mark_release_pending(prev_attempt);
                        return Err(CallError::ReleasePending);
                    }
                }
            }
        }

        target.calls().join(channel)
    }
}
