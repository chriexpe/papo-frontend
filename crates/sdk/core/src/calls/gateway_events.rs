//! Ordered once-only gateway voice event handlers for `SessionCalls`.
//! Enforces `peer_id` ownership, per-type route revisions and phase-aware error classification.

use crate::{Generation, Uuid};
use papo_api::gateway::{Command, Event, KnownEvent};
use std::sync::Arc;
use tokio::time::Instant;

use super::{
    coordinator::{CoordinatorState, SessionCalls, SideEffects},
    errors::{VoiceErrorCode, VoiceErrorDisposition, classify_voice_error},
    fifo::QueuedVoiceCommand,
    media_port::MediaCommand,
    model::{CallEndReason, CallPhase},
};

impl SessionCalls {
    /// Processes one incoming gateway event in strict socket order straight from the receive path.
    pub(crate) fn on_gateway_event(
        self: &Arc<Self>,
        generation: Generation,
        event: &Event,
        runtime: &tokio::runtime::Handle,
    ) {
        let Event::Known(known) = event else {
            return;
        };
        let mut effects = SideEffects::default();
        {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if guard.generation != generation {
                return;
            }
            match known {
                KnownEvent::PresenceSync(payload) => {
                    guard.reducer.apply_presence_sync(payload);
                    self.stage_rooms_publish(&mut guard, &mut effects);
                    self.stage_active_publish(&mut guard, &mut effects);
                }
                KnownEvent::VoiceJoined(payload) => {
                    self.handle_voice_joined_locked(&mut guard, payload, &mut effects);
                }
                KnownEvent::VoiceAnswer(payload) => {
                    self.handle_voice_answer_locked(&mut guard, payload, &mut effects);
                }
                KnownEvent::VoiceOffer(payload) => {
                    Self::handle_voice_offer_locked(&mut guard, payload, &mut effects);
                }
                KnownEvent::VoiceICECandidate(payload) => {
                    Self::handle_voice_ice_locked(&mut guard, payload, &mut effects);
                }
                KnownEvent::VoiceStateUpdate(payload) => {
                    if let Some((channel, _)) = guard.reducer.apply_state_update(payload) {
                        self.stage_rooms_publish(&mut guard, &mut effects);
                        if guard
                            .active
                            .as_ref()
                            .is_some_and(|active| active.channel == channel)
                        {
                            self.reconcile_subscriptions_locked(&mut guard, &mut effects);
                            self.stage_active_publish(&mut guard, &mut effects);
                        }
                    }
                }
                KnownEvent::VoiceLeave(payload) => {
                    self.handle_voice_leave_locked(&mut guard, payload, &mut effects);
                }
                KnownEvent::ActiveSpeakerUpdate(payload) => {
                    if let Some(channel) = guard.reducer.apply_active_speakers(payload) {
                        self.stage_rooms_publish(&mut guard, &mut effects);
                        if guard
                            .active
                            .as_ref()
                            .is_some_and(|active| active.channel == channel)
                        {
                            self.stage_active_publish(&mut guard, &mut effects);
                        }
                    }
                }
                KnownEvent::VoiceAudioRoutes(payload) => {
                    let state = &mut *guard;
                    let peer_ref = Self::active_peer_ref(state.active.as_ref());
                    if let Some((revision, routes)) =
                        state.reducer.apply_audio_routes(peer_ref, payload)
                    {
                        effects
                            .media_commands
                            .push(MediaCommand::SetAudioRoutes { revision, routes });
                        self.stage_active_publish(state, &mut effects);
                    }
                }
                KnownEvent::VoiceVideoRoutes(payload) => {
                    let state = &mut *guard;
                    let peer_ref = Self::active_peer_ref(state.active.as_ref());
                    if state
                        .reducer
                        .apply_video_routes(peer_ref, payload)
                        .is_some()
                    {
                        if let Some(active) = state.active.as_mut() {
                            active.subscriptions.note_video_routes(&state.reducer);
                            let (revision, slots) = active.slot_plan(&state.reducer);
                            effects
                                .media_commands
                                .push(MediaCommand::SetVideoSlotPlan { revision, slots });
                        }
                        self.stage_active_publish(state, &mut effects);
                    }
                }
                KnownEvent::Error(payload) => {
                    self.handle_voice_error_locked(&mut guard, payload, &mut effects);
                }
                _ => return,
            }
        }
        self.execute_effects(effects, runtime);
    }

    fn handle_voice_joined_locked(
        &self,
        guard: &mut CoordinatorState,
        payload: &papo_api::gateway::VoiceJoinedEvent,
        effects: &mut SideEffects,
    ) {
        let Ok(channel) = payload.channel_id.parse::<Uuid>() else {
            return;
        };
        if let Some(count) = guard.abandoned_joins.get_mut(&channel)
            && *count > 0
        {
            *count -= 1;
            if *count == 0 {
                guard.abandoned_joins.remove(&channel);
            }
            return;
        }
        let Some(active_channel) = guard
            .active
            .as_ref()
            .filter(|active| active.phase == CallPhase::Joining)
            .map(|active| active.channel)
        else {
            return;
        };
        let Some(admission) = guard.reducer.apply_voice_joined(active_channel, payload) else {
            return;
        };
        if let Some(active) = guard.active.as_mut() {
            active.phase = CallPhase::Joined;
            active.peer_id = Some(admission.peer_id);
            active.video_codec = Some(admission.video_codec);
            if !active.settings.muted {
                effects.wire_commands.push(QueuedVoiceCommand {
                    generation: active.generation,
                    attempt: active.attempt,
                    command: Command::VoiceMute {
                        channel_id: active.channel.to_string(),
                        muted: false,
                    },
                    allow_after_end: false,
                });
            }
            if active.settings.camera_on {
                effects.wire_commands.push(QueuedVoiceCommand {
                    generation: active.generation,
                    attempt: active.attempt,
                    command: Command::VoiceCamera {
                        channel_id: active.channel.to_string(),
                        on: true,
                    },
                    allow_after_end: false,
                });
            }
            effects.media_commands.push(MediaCommand::StartNegotiation {
                video_codec: admission.video_codec,
            });
        }
        self.stage_rooms_publish(guard, effects);
        self.stage_active_publish(guard, effects);
    }

    fn handle_voice_answer_locked(
        &self,
        guard: &mut CoordinatorState,
        payload: &papo_api::gateway::VoiceAnswerEvent,
        effects: &mut SideEffects,
    ) {
        let Some(peer) = Self::active_peer_ref(guard.active.as_ref()) else {
            return;
        };
        if payload.channel_id.parse::<Uuid>().ok() != Some(peer.channel)
            || payload.peer_id != peer.peer_id
        {
            return;
        }
        effects.media_commands.push(MediaCommand::RemoteAnswer {
            sdp: payload.sdp.clone(),
        });
        if let Some(active) = guard.active.as_mut() {
            active.subscriptions.mark_first_answer_received();
        }
        self.reconcile_subscriptions_locked(guard, effects);
    }

    fn handle_voice_offer_locked(
        guard: &mut CoordinatorState,
        payload: &papo_api::gateway::VoiceOfferEvent,
        effects: &mut SideEffects,
    ) {
        let Some(peer) = Self::active_peer_ref(guard.active.as_ref()) else {
            return;
        };
        if payload.channel_id.parse::<Uuid>().ok() != Some(peer.channel)
            || payload.peer_id != peer.peer_id
        {
            return;
        }
        effects.media_commands.push(MediaCommand::RemoteOffer {
            sdp: payload.sdp.clone(),
        });
    }

    fn handle_voice_ice_locked(
        guard: &mut CoordinatorState,
        payload: &papo_api::gateway::VoiceICECandidateEvent,
        effects: &mut SideEffects,
    ) {
        let Some(peer) = Self::active_peer_ref(guard.active.as_ref()) else {
            return;
        };
        if payload.channel_id.parse::<Uuid>().ok() != Some(peer.channel)
            || payload.peer_id != peer.peer_id
        {
            return;
        }
        effects
            .media_commands
            .push(MediaCommand::RemoteIceCandidate {
                candidate: payload.candidate.clone(),
                sdp_mid: payload.sdp_mid.clone(),
                sdp_mline_index: payload.sdp_mline_index,
            });
    }

    fn handle_voice_leave_locked(
        &self,
        guard: &mut CoordinatorState,
        payload: &papo_api::gateway::VoiceLeaveEvent,
        effects: &mut SideEffects,
    ) {
        let peer_ref = Self::active_peer_ref(guard.active.as_ref());
        let outcome = guard
            .reducer
            .apply_leave(self.account.user(), peer_ref, payload);
        if outcome.ended_active_call {
            self.finish_active_locked(guard, CallEndReason::ServerRemoved, false, effects);
        } else if let Ok(user) = payload.user_id.parse::<Uuid>()
            && let Some(active) = guard.active.as_mut()
            && payload.channel_id.parse::<Uuid>().ok() == Some(active.channel)
        {
            active.subscriptions.note_member_left(user);
            if outcome.video_routes_changed {
                let (revision, slots) = active.slot_plan(&guard.reducer);
                effects
                    .media_commands
                    .push(MediaCommand::SetVideoSlotPlan { revision, slots });
            }
            self.reconcile_subscriptions_locked(guard, effects);
            self.stage_active_publish(guard, effects);
        }
        if outcome.rooms_changed {
            self.stage_rooms_publish(guard, effects);
        }
    }

    fn handle_voice_error_locked(
        &self,
        guard: &mut CoordinatorState,
        payload: &papo_api::gateway::ErrorEvent,
        effects: &mut SideEffects,
    ) {
        let Some(code) = payload.code.as_deref().and_then(VoiceErrorCode::from_wire) else {
            return;
        };
        if let Some(peer_id) = payload.peer_id.as_deref().filter(|id| !id.is_empty()) {
            let Some(active_peer) = Self::active_peer_ref(guard.active.as_ref()) else {
                return;
            };
            if peer_id != active_peer.peer_id {
                return;
            }
        }
        if let Some(channel_str) = payload.channel_id.as_deref().filter(|id| !id.is_empty()) {
            let Ok(channel) = channel_str.parse::<Uuid>() else {
                return;
            };
            if guard
                .active
                .as_ref()
                .is_none_or(|active| active.channel != channel)
            {
                return;
            }
        }
        let has_peer_scope = payload.peer_id.as_deref().is_some_and(|id| !id.is_empty());
        if !has_peer_scope
            && matches!(
                classify_voice_error(code, true),
                VoiceErrorDisposition::Fatal(_)
            )
            && !guard.abandoned_joins.is_empty()
            && let Some(first_channel) = guard.abandoned_joins.keys().next().copied()
            && let Some(count) = guard.abandoned_joins.get_mut(&first_channel)
        {
            *count = count.saturating_sub(1);
            if *count == 0 {
                guard.abandoned_joins.remove(&first_channel);
            }
            return;
        }

        let Some(active) = guard.active.as_mut() else {
            return;
        };
        if !active.phase.is_active() {
            return;
        }
        let joining = active.phase == CallPhase::Joining;
        match classify_voice_error(code, joining) {
            VoiceErrorDisposition::Fatal(reason) => {
                let send_leave = !joining
                    && !matches!(reason, CallEndReason::RoomClosed | CallEndReason::Forbidden);
                self.finish_active_locked(guard, reason, send_leave, effects);
            }
            VoiceErrorDisposition::NonFatal {
                warning,
                retry_subscriptions,
            } => {
                if let Some(warning) = warning {
                    active.push_warning(warning);
                }
                if retry_subscriptions {
                    active
                        .subscriptions
                        .note_retryable_rejection(&guard.reducer, Instant::now());
                    if let Some(at) = active.subscriptions.next_wakeup(
                        active.channel,
                        self.account.user(),
                        &guard.reducer,
                    ) {
                        effects.schedule_retry_at = Some((active.attempt, at));
                    }
                }
                self.stage_active_publish(guard, effects);
            }
        }
    }
}
