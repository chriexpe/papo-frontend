//! Attempt-scoped call handles, controls, `CallView` subscriptions, and Rust video binding hook.
//!
//! `FfiCallHandle::bind_video_destination` allows Session 16's single combined Android `.so`
//! (`papo-ffi` rlib + `papo-call` + JNI `ANativeWindow` surface bridge) to bind directly to the
//! exact active `CallHandle` instance without duplicating client ownership or crossing `cdylib`s.

use std::sync::Arc;

use papo_sdk::{CallHandle, VideoDestination};
use papo_sdk_ui::CallService;

use crate::{
    call_enums::{FfiCameraFacing, FfiVideoKind, FfiVoiceCapability},
    call_snapshot::{FfiActiveAccountCall, FfiCallSnapshot, FfiVideoTarget},
    call_view::FfiCallView,
    client::FfiClient,
    errors::FfiClientError,
    ids::{FfiAccountId, parse_uuid},
    operation_errors::FfiCallError,
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription, watch_stream},
};

/// Listener for single-attempt `FfiCallHandle` state snapshots ending at `CallPhase::Ended`.
#[uniffi::export(with_foreign)]
pub trait FfiCallAttemptListener: Send + Sync {
    /// Atomic initial snapshot of this call attempt.
    fn on_snapshot(&self, snapshot: FfiCallSnapshot);
    /// Subsequent updates for this attempt up to and including `Ended`.
    fn on_update(&self, snapshot: FfiCallSnapshot);
    /// Terminal notification after attempt stream finishes.
    fn on_closed(&self);
}

/// Listener for joined account `FfiCallView` (active call + admitted voice rooms).
#[uniffi::export(with_foreign)]
pub trait FfiCallViewListener: Send + Sync {
    /// Atomic initial projection.
    fn on_snapshot(&self, view: FfiCallView);
    /// Subsequent updates across room occupancy, call state, and admitted names/avatars.
    fn on_update(&self, view: FfiCallView);
    /// Terminal notification when session closes.
    fn on_closed(&self);
}

/// Handle scoped to one active call attempt; permanently inert once that attempt ends.
#[derive(uniffi::Object)]
pub struct FfiCallHandle {
    /// Parent FFI client for subscription spawning.
    client: Arc<FfiClient>,
    /// Underlying SDK attempt-scoped handle.
    handle: CallHandle,
}

impl FfiCallHandle {
    /// Wraps an SDK `CallHandle` tied to `client`.
    pub(crate) fn from_sdk(client: Arc<FfiClient>, handle: CallHandle) -> Arc<Self> {
        Arc::new(Self { client, handle })
    }

    /// Returns the underlying SDK [`CallHandle`] for Session 16 same-image native media bridging.
    #[must_use]
    pub fn inner(&self) -> &CallHandle {
        &self.handle
    }

    /// Binds a host video destination (`papo-call` surface sink) directly to `(participant, kind)`
    /// on this exact active attempt handle inside Session 16's single native `.so`.
    ///
    /// # Errors
    /// Returns [`FfiCallError::InvalidArgument`] for malformed `participant` or [`FfiCallError::Ended`]
    /// if this attempt already terminated.
    pub fn bind_video_destination(
        &self,
        participant: String,
        kind: FfiVideoKind,
        destination: Arc<dyn VideoDestination>,
    ) -> Result<(), FfiCallError> {
        let user = parse_uuid(&participant).map_err(|_| FfiCallError::InvalidArgument)?;
        self.handle
            .bind_video(user, kind.into(), destination)
            .map_err(Into::into)
    }
}

#[uniffi::export]
impl FfiCallHandle {
    /// Account owning this call attempt.
    #[must_use]
    pub fn account(&self) -> FfiAccountId {
        FfiAccountId::from_sdk(self.handle.account())
    }

    /// Unique monotonic attempt identifier for foreground-service and audio-focus ownership checks.
    #[must_use]
    pub fn attempt_id(&self) -> u64 {
        self.handle.attempt().raw()
    }

    /// Current attempt state snapshot without blocking.
    #[must_use]
    pub fn snapshot(&self) -> FfiCallSnapshot {
        (&self.handle.snapshot()).into()
    }

    /// Subscribes atomically to attempt snapshots up through terminal `Ended`.
    ///
    /// # Errors
    /// Returns [`FfiCallError::Closed`] if client shut down or worker creation failed.
    pub fn subscribe(
        &self,
        listener: Arc<dyn FfiCallAttemptListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiCallError> {
        let (baseline, stream) = watch_stream(self.handle.subscribe());
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.client.runtime,
            &self.client.tasks,
            &self.client.shutdown,
            self.client.callbacks.clone(),
            (&baseline).into(),
            stream,
            move |snap| {
                l_snap.on_snapshot(snap);
                true
            },
            move |snap| {
                l_upd.on_update((&snap).into());
                true
            },
            move || l_end.on_closed(),
        )
        .map_err(|_| FfiCallError::Closed)
    }

    /// Leaves this call attempt; safe and idempotent on ended handles.
    pub fn leave(&self) {
        self.handle.leave();
    }

    /// Sets local microphone mute intent.
    ///
    /// # Errors
    /// Returns [`FfiCallError::Ended`] once the attempt ended.
    pub fn set_muted(&self, muted: bool) -> Result<(), FfiCallError> {
        self.handle.set_muted(muted).map_err(Into::into)
    }

    /// Sets local camera publish intent (`voice_camera` ordered before offer).
    ///
    /// # Errors
    /// Returns [`FfiCallError::Ended`] once the attempt ended.
    pub fn set_camera(&self, on: bool) -> Result<(), FfiCallError> {
        self.handle.set_camera(on).map_err(Into::into)
    }

    /// Sets local camera facing direction (`Front` / `Back`).
    ///
    /// # Errors
    /// Returns [`FfiCallError::Ended`] once the attempt ended.
    pub fn set_camera_facing(&self, facing: FfiCameraFacing) -> Result<(), FfiCallError> {
        self.handle
            .set_camera_facing(facing.into())
            .map_err(Into::into)
    }

    /// Sets independent local receiver gain (`0..=200`) and mute for `user_id`.
    ///
    /// # Errors
    /// Returns [`FfiCallError::InvalidArgument`] or [`FfiCallError::Ended`].
    pub fn set_playback(
        &self,
        user_id: String,
        gain_percent: u16,
        muted: bool,
    ) -> Result<(), FfiCallError> {
        let user = parse_uuid(&user_id).map_err(|_| FfiCallError::InvalidArgument)?;
        self.handle
            .set_playback(user, gain_percent, muted)
            .map_err(Into::into)
    }

    /// Sets ordered remote video streams wanted by the UI.
    ///
    /// # Errors
    /// Returns [`FfiCallError::InvalidArgument`] or [`FfiCallError::Ended`].
    pub fn set_video_interest(&self, targets: Vec<FfiVideoTarget>) -> Result<(), FfiCallError> {
        let sdk_targets = targets
            .into_iter()
            .map(FfiVideoTarget::into_sdk)
            .collect::<Result<Vec<_>, _>>()?;
        self.handle
            .set_video_interest(sdk_targets)
            .map_err(Into::into)
    }

    /// Detaches and revokes any host video destination bound to `(participant_user_id, kind)`.
    ///
    /// # Errors
    /// Returns [`FfiCallError::InvalidArgument`] or [`FfiCallError::Ended`].
    pub fn unbind_video(
        &self,
        participant_user_id: String,
        kind: FfiVideoKind,
    ) -> Result<(), FfiCallError> {
        let user = parse_uuid(&participant_user_id).map_err(|_| FfiCallError::InvalidArgument)?;
        self.handle
            .unbind_video(user, kind.into())
            .map_err(Into::into)
    }

    /// Host callback: OS audio focus lost (mutes only this attempt).
    ///
    /// # Errors
    /// Returns [`FfiCallError::Ended`] once ended.
    pub fn on_audio_focus_lost(&self) -> Result<(), FfiCallError> {
        self.handle.on_audio_focus_lost().map_err(Into::into)
    }

    /// Host callback: OS audio focus regained (never unmutes automatically).
    ///
    /// # Errors
    /// Returns [`FfiCallError::Ended`] once ended.
    pub fn on_audio_focus_regained(&self) -> Result<(), FfiCallError> {
        self.handle.on_audio_focus_regained().map_err(Into::into)
    }

    /// Host callback: OS terminated foreground service (ends only this attempt).
    ///
    /// # Errors
    /// Returns [`FfiCallError::Ended`] once ended.
    pub fn on_host_terminated(&self) -> Result<(), FfiCallError> {
        self.handle.on_host_terminated().map_err(Into::into)
    }
}

/// Maps client session lookup failure into [`FfiCallError`].
fn call_session(
    client: &FfiClient,
    account: &FfiAccountId,
) -> Result<papo_sdk::Session, FfiCallError> {
    client.session_by_id(account).map_err(|err| match err {
        FfiClientError::InvalidArgument => FfiCallError::InvalidArgument,
        FfiClientError::Closed => FfiCallError::Closed,
        _ => FfiCallError::AccountNotFound,
    })
}

#[uniffi::export]
impl FfiClient {
    /// Returns current online `connect_voice` projection for `channel_id` on `account`.
    ///
    /// # Errors
    /// Returns invalid argument or account session error.
    pub fn connect_voice_capability(
        &self,
        account: FfiAccountId,
        channel_id: String,
    ) -> Result<FfiVoiceCapability, FfiCallError> {
        let session = call_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiCallError::InvalidArgument)?;
        Ok(CallService::new(session).connect_voice(channel).into())
    }

    /// Joins `channel_id` on `account`, returning an attempt-scoped handle without blocking.
    ///
    /// # Errors
    /// Returns precondition or lease failures (`Busy`, `Offline`, `Forbidden`, `MediaUnavailable`, ...).
    pub fn join_call(
        self: &Arc<Self>,
        account: FfiAccountId,
        channel_id: String,
    ) -> Result<Arc<FfiCallHandle>, FfiCallError> {
        let session = call_session(self, &account)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiCallError::InvalidArgument)?;
        let handle = CallService::new(session).join(channel)?;
        Ok(FfiCallHandle::from_sdk(Arc::clone(self), handle))
    }

    /// Reattaches to `account`'s active call handle after navigation or view rebuild, if any.
    ///
    /// # Errors
    /// Returns invalid argument or account session error.
    pub fn active_account_call(
        self: &Arc<Self>,
        account: FfiAccountId,
    ) -> Result<Option<Arc<FfiCallHandle>>, FfiCallError> {
        let session = call_session(self, &account)?;
        Ok(CallService::new(session)
            .active()
            .map(|h| FfiCallHandle::from_sdk(Arc::clone(self), h)))
    }

    /// Returns the currently active call across all accounts on this client, if any.
    #[must_use]
    pub fn active_call(self: &Arc<Self>) -> Option<FfiActiveAccountCall> {
        self.handle
            .active_call()
            .map(|(account, handle)| FfiActiveAccountCall {
                account: FfiAccountId::from_sdk(account),
                handle: FfiCallHandle::from_sdk(Arc::clone(self), handle),
            })
    }

    /// Transfers client-wide media lease cleanly from any prior account call to `(account, channel_id)`.
    ///
    /// # Errors
    /// Returns target precondition, prior drain timeout (`ReleasePending`), or lifecycle failures.
    pub async fn switch_call(
        self: &Arc<Self>,
        account: FfiAccountId,
        channel_id: String,
    ) -> Result<Arc<FfiCallHandle>, FfiCallError> {
        let sdk_account = account
            .into_sdk()
            .map_err(|_| FfiCallError::InvalidArgument)?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiCallError::InvalidArgument)?;
        let client_handle = self.handle.clone();
        let ffi_client = Arc::clone(self);
        self.run_scoped(|| FfiCallError::Closed, async move {
            let handle = client_handle.switch_call(sdk_account, channel).await?;
            Ok(FfiCallHandle::from_sdk(ffi_client, handle))
        })
        .await
    }

    /// Subscribes atomically to voice rooms and active call state joined with admitted member profiles.
    ///
    /// # Errors
    /// Returns missing session, invalid ID, or worker start error.
    pub fn subscribe_calls(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiCallViewListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiCallError> {
        let session = call_session(self, &account)?;
        let sub = CallService::new(session).subscribe();
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.runtime,
            &self.tasks,
            &self.shutdown,
            self.callbacks.clone(),
            (&sub.snapshot).into(),
            sub.stream,
            move |view| {
                l_snap.on_snapshot(view);
                true
            },
            move |view| {
                l_upd.on_update((&view).into());
                true
            },
            move || l_end.on_closed(),
        )
        .map_err(|_| FfiCallError::Closed)
    }
}
