//! Ordered once-only gateway voice event handlers for `SessionCalls`.
//! Enforces `peer_id` ownership, per-type route revisions and phase-aware error classification.

use crate::{Generation, Uuid};
use papo_api::gateway::{Command, Event, KnownEvent};
use std::sync::Arc;
use tokio::time::Instant;

use super::{
    coordinator::{CoordinatorState, JoinDispatchState, SessionCalls, SideEffects},
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
                    self.publish_rooms_locked(&mut guard);
                    self.publish_active_locked(&mut guard);
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
                        self.publish_rooms_locked(&mut guard);
                        if guard
                            .active
                            .as_ref()
                            .is_some_and(|active| active.channel == channel)
                        {
                            self.reconcile_subscriptions_locked(&mut guard, &mut effects);
                            self.publish_active_locked(&mut guard);
                        }
                    }
                }
                KnownEvent::VoiceLeave(payload) => {
                    self.handle_voice_leave_locked(&mut guard, payload, &mut effects);
                }
                KnownEvent::ActiveSpeakerUpdate(payload) => {
                    if let Some(channel) = guard.reducer.apply_active_speakers(payload) {
                        self.publish_rooms_locked(&mut guard);
                        if guard
                            .active
                            .as_ref()
                            .is_some_and(|active| active.channel == channel)
                        {
                            self.publish_active_locked(&mut guard);
                        }
                    }
                }
                KnownEvent::VoiceAudioRoutes(payload) => {
                    let state = &mut *guard;
                    let peer_ref = Self::active_peer_ref(state.active.as_ref());
                    if let Some((revision, routes)) =
                        state.reducer.apply_audio_routes(peer_ref, payload)
                    {
                        Self::enqueue_media_locked(
                            state,
                            MediaCommand::SetAudioRoutes { revision, routes },
                            &mut effects,
                        );
                        self.publish_active_locked(state);
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
                            let (revision, slots) = active.next_slot_plan(&state.reducer);
                            Self::enqueue_media_locked(
                                state,
                                MediaCommand::SetVideoSlotPlan { revision, slots },
                                &mut effects,
                            );
                        }
                        self.publish_active_locked(state);
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
        if let Some(pos) = guard.abandoned_joins.iter().position(|ch| *ch == channel) {
            guard.abandoned_joins.remove(pos);
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
        let mut initial_wire = Vec::new();
        if let Some(active) = guard.active.as_mut() {
            active.phase = CallPhase::Joined;
            active.join_state = JoinDispatchState::Acknowledged;
            active.peer_id = Some(admission.peer_id);
            active.video_codec = Some(admission.video_codec);
            if !active.settings.muted {
                initial_wire.push(QueuedVoiceCommand {
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
                initial_wire.push(QueuedVoiceCommand {
                    generation: active.generation,
                    attempt: active.attempt,
                    command: Command::VoiceCamera {
                        channel_id: active.channel.to_string(),
                        on: true,
                    },
                    allow_after_end: false,
                });
            }
        }
        for item in initial_wire {
            if !self.enqueue_wire_locked(guard, item, effects) {
                self.publish_rooms_locked(guard);
                return;
            }
        }
        Self::enqueue_media_locked(
            guard,
            MediaCommand::StartNegotiation {
                video_codec: admission.video_codec,
            },
            effects,
        );
        self.publish_rooms_locked(guard);
        self.publish_active_locked(guard);
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
        if let Some(active) = guard.active.as_mut() {
            active.subscriptions.mark_first_answer_received();
        }
        Self::enqueue_media_locked(
            guard,
            MediaCommand::RemoteAnswer {
                sdp: payload.sdp.clone(),
            },
            effects,
        );
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
        Self::enqueue_media_locked(
            guard,
            MediaCommand::RemoteOffer {
                sdp: payload.sdp.clone(),
            },
            effects,
        );
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
        Self::enqueue_media_locked(
            guard,
            MediaCommand::RemoteIceCandidate {
                candidate: payload.candidate.clone(),
                sdp_mid: payload.sdp_mid.clone(),
                sdp_mline_index: payload.sdp_mline_index,
            },
            effects,
        );
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
            && guard.active.as_ref().is_some_and(|active| {
                payload.channel_id.parse::<Uuid>().ok() == Some(active.channel)
            })
        {
            if let Some(active) = guard.active.as_mut() {
                active.subscriptions.note_member_left(user);
            }
            if outcome.routes.audio_changed {
                let revision = guard.reducer.audio_revision();
                let routes = guard.reducer.audio_route_vec();
                Self::enqueue_media_locked(
                    guard,
                    MediaCommand::SetAudioRoutes { revision, routes },
                    effects,
                );
            }
            if outcome.routes.video_changed
                && let Some(active) = guard.active.as_mut()
            {
                let (revision, slots) = active.next_slot_plan(&guard.reducer);
                Self::enqueue_media_locked(
                    guard,
                    MediaCommand::SetVideoSlotPlan { revision, slots },
                    effects,
                );
            }
            self.reconcile_subscriptions_locked(guard, effects);
            self.publish_active_locked(guard);
        }
        if outcome.rooms_changed {
            self.publish_rooms_locked(guard);
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
        let has_peer_scope = payload.peer_id.as_deref().is_some_and(|id| !id.is_empty());
        if let Some(peer_id) = payload.peer_id.as_deref().filter(|id| !id.is_empty()) {
            let Some(active_peer) = Self::active_peer_ref(guard.active.as_ref()) else {
                return;
            };
            if peer_id != active_peer.peer_id {
                return;
            }
        }

        let err_channel = match payload
            .channel_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .map(str::parse::<Uuid>)
        {
            Some(Ok(ch)) => Some(ch),
            Some(Err(_)) => return,
            None => None,
        };

        // Consume a join-phase error belonging to an abandoned join without swallowing a real
        // channel-scoped error (such as `voice-room-closed`) for an active Joined/Connected call.
        if !has_peer_scope
            && matches!(
                classify_voice_error(code, true),
                VoiceErrorDisposition::Fatal(_)
            )
            && !guard.abandoned_joins.is_empty()
        {
            if let Some(ch) = err_channel {
                let active_live_on_ch = guard.active.as_ref().is_some_and(|active| {
                    active.channel == ch
                        && matches!(active.phase, CallPhase::Joined | CallPhase::Connected)
                });
                if !active_live_on_ch
                    && let Some(pos) = guard.abandoned_joins.iter().position(|entry| *entry == ch)
                {
                    guard.abandoned_joins.remove(pos);
                    return;
                }
            } else {
                guard.abandoned_joins.pop_front();
                return;
            }
        }

        if let Some(ch) = err_channel
            && guard
                .active
                .as_ref()
                .is_none_or(|active| active.channel != ch)
        {
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
                        Self::stage_subscription_wakeup_locked(active, at, effects);
                    }
                }
                self.publish_active_locked(guard);
            }
        }
    }
}
