//! Attempt-fenced control operations backing `CallHandle` and `HostLease`.

use crate::Uuid;
use papo_api::gateway::Command;
use std::sync::Arc;

use super::{
    coordinator::{CoordinatorState, SessionCalls, SideEffects},
    fifo::QueuedVoiceCommand,
    media_port::{MediaCommand, VideoDestination},
    model::{
        CallAttemptId, CallEndReason, CallError, CallPhase, CameraFacing, HostLeaseKind,
        PlaybackSettings, VideoKind, VideoTarget,
    },
};

fn control_attempt(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    runtime: &tokio::runtime::Handle,
    action: impl FnOnce(
        &Arc<SessionCalls>,
        &mut CoordinatorState,
        &mut SideEffects,
    ) -> Result<(), CallError>,
) -> Result<(), CallError> {
    let mut effects = SideEffects::default();
    let result = {
        let mut guard = calls
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !guard
            .active
            .as_ref()
            .is_some_and(|active| active.attempt == attempt && active.phase.is_active())
        {
            return Err(CallError::Ended);
        }
        action(calls, &mut guard, &mut effects)
    };
    calls.execute_effects(effects, runtime);
    result
}

/// Updates mute intent on the active attempt and notifies media + server.
pub(crate) fn set_muted(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    muted: bool,
    runtime: &tokio::runtime::Handle,
) -> Result<(), CallError> {
    control_attempt(calls, attempt, runtime, |this, guard, effects| {
        let active = guard.active.as_mut().ok_or(CallError::Ended)?;
        active.settings.muted = muted;
        effects
            .media_commands
            .push(MediaCommand::SetMuted { muted });
        if matches!(active.phase, CallPhase::Joined | CallPhase::Connected) {
            effects.wire_commands.push(QueuedVoiceCommand {
                generation: active.generation,
                attempt: active.attempt,
                command: Command::VoiceMute {
                    channel_id: active.channel.to_string(),
                    muted,
                },
                allow_after_end: false,
            });
        }
        this.stage_active_publish(guard, effects);
        Ok(())
    })
}

/// Updates camera intent, queuing `voice_camera` ahead of any renegotiation offer.
pub(crate) fn set_camera(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    on: bool,
    runtime: &tokio::runtime::Handle,
) -> Result<(), CallError> {
    control_attempt(calls, attempt, runtime, |this, guard, effects| {
        let active = guard.active.as_mut().ok_or(CallError::Ended)?;
        active.settings.camera_on = on;
        if matches!(active.phase, CallPhase::Joined | CallPhase::Connected) {
            effects.wire_commands.push(QueuedVoiceCommand {
                generation: active.generation,
                attempt: active.attempt,
                command: Command::VoiceCamera {
                    channel_id: active.channel.to_string(),
                    on,
                },
                allow_after_end: false,
            });
        }
        effects.media_commands.push(MediaCommand::SetCamera { on });
        this.stage_active_publish(guard, effects);
        Ok(())
    })
}

/// Updates desired camera facing direction.
pub(crate) fn set_camera_facing(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    facing: CameraFacing,
    runtime: &tokio::runtime::Handle,
) -> Result<(), CallError> {
    control_attempt(calls, attempt, runtime, |this, guard, effects| {
        let active = guard.active.as_mut().ok_or(CallError::Ended)?;
        active.settings.camera_facing = facing;
        effects
            .media_commands
            .push(MediaCommand::SetCameraFacing { facing });
        this.stage_active_publish(guard, effects);
        Ok(())
    })
}

/// Sets independent per-user receiver gain (`0..=200`) and mute without changing speaking state.
pub(crate) fn set_playback(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    user: Uuid,
    gain_percent: u16,
    muted: bool,
    runtime: &tokio::runtime::Handle,
) -> Result<(), CallError> {
    control_attempt(calls, attempt, runtime, |this, guard, effects| {
        let active = guard.active.as_mut().ok_or(CallError::Ended)?;
        let playback = PlaybackSettings::new(gain_percent, muted);
        active.playback.insert(user, playback);
        effects
            .media_commands
            .push(MediaCommand::SetPlayback { user, playback });
        this.stage_active_publish(guard, effects);
        Ok(())
    })
}

/// Updates ordered UI video interest and reconciles `track_subscribe` / `track_unsubscribe`.
pub(crate) fn set_video_interest(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    targets: Vec<VideoTarget>,
    runtime: &tokio::runtime::Handle,
) -> Result<(), CallError> {
    control_attempt(calls, attempt, runtime, |this, guard, effects| {
        let active = guard.active.as_mut().ok_or(CallError::Ended)?;
        active.subscriptions.set_interest(targets);
        this.reconcile_subscriptions_locked(guard, effects);
        Ok(())
    })
}

/// Binds a host video destination to `(participant, kind)` and updates the active video slot plan.
pub(crate) fn bind_video(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    participant: Uuid,
    kind: VideoKind,
    destination: Arc<dyn VideoDestination>,
    runtime: &tokio::runtime::Handle,
) -> Result<(), CallError> {
    control_attempt(calls, attempt, runtime, |this, guard, effects| {
        let active = guard.active.as_mut().ok_or(CallError::Ended)?;
        let new_id = destination.id();
        if let Some(old) = active.destinations.insert((participant, kind), destination)
            && old.id() != new_id
        {
            effects.revoke_destinations.push(old);
        }
        let (revision, slots) = active.slot_plan(&guard.reducer);
        effects
            .media_commands
            .push(MediaCommand::SetVideoSlotPlan { revision, slots });
        this.stage_active_publish(guard, effects);
        Ok(())
    })
}

/// Unbinds and revokes any host video destination for `(participant, kind)`.
pub(crate) fn unbind_video(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    participant: Uuid,
    kind: VideoKind,
    runtime: &tokio::runtime::Handle,
) -> Result<(), CallError> {
    control_attempt(calls, attempt, runtime, |this, guard, effects| {
        let active = guard.active.as_mut().ok_or(CallError::Ended)?;
        if let Some(old) = active.destinations.remove(&(participant, kind)) {
            effects.revoke_destinations.push(old);
        }
        let (revision, slots) = active.slot_plan(&guard.reducer);
        effects
            .media_commands
            .push(MediaCommand::SetVideoSlotPlan { revision, slots });
        this.stage_active_publish(guard, effects);
        Ok(())
    })
}

/// Records host audio focus loss: mutes the active call attempt; stale attempts are ignored.
pub(crate) fn on_audio_focus_lost(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    runtime: &tokio::runtime::Handle,
) -> Result<(), CallError> {
    control_attempt(calls, attempt, runtime, |this, guard, effects| {
        let active = guard.active.as_mut().ok_or(CallError::Ended)?;
        active.host_audio_focus = false;
        if !active.settings.muted {
            active.settings.muted = true;
            effects
                .media_commands
                .push(MediaCommand::SetMuted { muted: true });
            if matches!(active.phase, CallPhase::Joined | CallPhase::Connected) {
                effects.wire_commands.push(QueuedVoiceCommand {
                    generation: active.generation,
                    attempt: active.attempt,
                    command: Command::VoiceMute {
                        channel_id: active.channel.to_string(),
                        muted: true,
                    },
                    allow_after_end: false,
                });
            }
        }
        this.stage_active_publish(guard, effects);
        Ok(())
    })
}

/// Records host audio focus regain without ever unmuting the microphone.
pub(crate) fn on_audio_focus_regained(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    runtime: &tokio::runtime::Handle,
) -> Result<(), CallError> {
    control_attempt(calls, attempt, runtime, |this, guard, effects| {
        let active = guard.active.as_mut().ok_or(CallError::Ended)?;
        active.host_audio_focus = true;
        this.stage_active_publish(guard, effects);
        Ok(())
    })
}

/// Ends this specific attempt if it is still current; stale attempts are untouched.
pub(crate) fn leave_attempt(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    reason: CallEndReason,
    runtime: &tokio::runtime::Handle,
) {
    let mut effects = SideEffects::default();
    {
        let mut guard = calls
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if guard
            .active
            .as_ref()
            .is_some_and(|active| active.attempt == attempt && active.phase.is_active())
        {
            calls.finish_active_locked(&mut guard, reason, true, &mut effects);
        }
    }
    calls.execute_effects(effects, runtime);
}

/// Acquires a compare-on-release host lease token scoped to `(attempt, kind)`.
pub(crate) fn acquire_host_lease(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    kind: HostLeaseKind,
) -> Result<u64, CallError> {
    let mut guard = calls
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let state = &mut *guard;
    let active = state
        .active
        .as_mut()
        .filter(|active| active.attempt == attempt && active.phase.is_active())
        .ok_or(CallError::Ended)?;
    let id = state.next_host_lease_id;
    state.next_host_lease_id = state.next_host_lease_id.saturating_add(1);
    active.host_leases.insert(kind, id);
    Ok(id)
}

/// Releases a host lease ONLY if `(attempt, kind, lease_id)` still matches the current attempt.
pub(crate) fn release_host_lease(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    kind: HostLeaseKind,
    lease_id: u64,
) -> bool {
    let mut guard = calls
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(active) = guard
        .active
        .as_mut()
        .filter(|active| active.attempt == attempt && active.phase.is_active())
    else {
        return false;
    };
    if active.host_leases.get(&kind).copied() == Some(lease_id) {
        active.host_leases.remove(&kind);
        true
    } else {
        false
    }
}
