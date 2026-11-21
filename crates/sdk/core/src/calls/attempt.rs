//! Active call attempt record, snapshot/slot-plan projection and deferred coordinator side effects.
//! Coordinator transitions and execution live in `coordinator.rs` and `lifecycle.rs`.

use crate::{AccountId, Generation, ResourceSnapshot, Uuid, observable::StateCell};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::Arc,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{
    fifo::VoiceCommandSender,
    media_port::{
        CaptureState, MediaCommand, MediaLease, MediaTransportState, VideoDestination,
        VideoSlotBinding,
    },
    model::{
        CallAttemptId, CallParticipant, CallPhase, CallSettings, CallSnapshot, CallWarning,
        PlaybackSettings, VideoBinding, VideoCodec, VideoKind, VideoTarget,
    },
    reducer::{ReducedVideoRoute, VoiceReducer},
    subscriptions::VideoSubscriptionPlanner,
};

/// Explicit wire lifecycle of this attempt's `voice_join` command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JoinDispatchState {
    /// `voice_join` has not yet been enqueued (`Preparing`).
    NotQueued,
    /// `voice_join` is queued in the voice FIFO but has not yet been approved for wire send.
    Queued,
    /// `voice_join` was approved for wire transmission by `AccountSocket`, awaiting `voice_joined`.
    Transmitted,
    /// `voice_joined` was received and accepted for this attempt.
    Acknowledged,
}

/// A locally ended join with an outstanding server-side leave.
pub(crate) struct AbandonedJoin {
    pub attempt: CallAttemptId,
    pub channel: Uuid,
    /// A rejected join can still produce a separate error for its ordered leave.
    pub join_rejected: bool,
    /// Known peer identity, including acknowledgements received after the attempt ended.
    pub peer_id: Option<String>,
    /// The writer approved the leave and no later join has been acknowledged.
    pub awaiting_leave_reply: bool,
}

impl AbandonedJoin {
    pub(crate) fn awaits_join_reply(&self) -> bool {
        !self.join_rejected && self.peer_id.is_none()
    }
}

/// Active attempt owned by a session.
pub(crate) struct ActiveAttempt {
    pub attempt: CallAttemptId,
    pub generation: Generation,
    pub channel: Uuid,
    pub phase: CallPhase,
    pub peer_id: Option<String>,
    pub video_codec: Option<VideoCodec>,
    pub settings: CallSettings,
    pub mic_capture: CaptureState,
    pub camera_capture: CaptureState,
    pub transport: MediaTransportState,
    pub warnings: Vec<CallWarning>,
    pub playback: BTreeMap<Uuid, PlaybackSettings>,
    pub destinations: BTreeMap<(Uuid, VideoKind), Arc<dyn VideoDestination>>,
    pub subscriptions: VideoSubscriptionPlanner,
    pub scheduled_wakeup: Option<Instant>,
    pub video_slot_plan_revision: u64,
    pub pending_media_commands: VecDeque<MediaCommand>,
    pub media_lease: Option<Arc<dyn MediaLease>>,
    pub media_start_in_flight: bool,
    pub join_state: JoinDispatchState,
    pub host_audio_focus: bool,
    pub cancel: CancellationToken,
    pub snapshot_cell: Arc<StateCell<CallSnapshot>>,
    pub version: u64,
}

impl ActiveAttempt {
    /// Constructs a new `Preparing` attempt and its initial `StateCell`.
    pub(crate) fn new(
        account: AccountId,
        generation: Generation,
        attempt: CallAttemptId,
        channel: Uuid,
        cancel: CancellationToken,
    ) -> (Self, Arc<StateCell<CallSnapshot>>) {
        let initial = CallSnapshot {
            version: 1,
            account,
            generation,
            attempt,
            channel,
            phase: CallPhase::Preparing,
            peer_id: None,
            video_codec: None,
            settings: CallSettings::default(),
            mic_capture: CaptureState::Inactive,
            camera_capture: CaptureState::Inactive,
            transport: MediaTransportState::New,
            warnings: Vec::new(),
            playback: BTreeMap::new(),
            participants: Vec::new(),
            audio_routes_known: false,
            audio_routes: BTreeMap::new(),
            video_bindings: Vec::new(),
            host_audio_focus: true,
        };
        let cell = StateCell::new(initial);
        let active = Self {
            attempt,
            generation,
            channel,
            phase: CallPhase::Preparing,
            peer_id: None,
            video_codec: None,
            settings: CallSettings::default(),
            mic_capture: CaptureState::Inactive,
            camera_capture: CaptureState::Inactive,
            transport: MediaTransportState::New,
            warnings: Vec::new(),
            playback: BTreeMap::new(),
            destinations: BTreeMap::new(),
            subscriptions: VideoSubscriptionPlanner::default(),
            scheduled_wakeup: None,
            video_slot_plan_revision: 0,
            pending_media_commands: VecDeque::new(),
            media_lease: None,
            media_start_in_flight: false,
            join_state: JoinDispatchState::NotQueued,
            host_audio_focus: true,
            cancel,
            snapshot_cell: Arc::clone(&cell),
            version: 1,
        };
        (active, cell)
    }

    /// Projects the current `CallSnapshot` for this attempt.
    pub(crate) fn snapshot(&self, account: AccountId, reducer: &VoiceReducer) -> CallSnapshot {
        let participants = reducer
            .room_members(self.channel)
            .into_iter()
            .map(|(user, device, speaking)| CallParticipant {
                account,
                attempt: self.attempt,
                user,
                device,
                speaking,
                playback: self.playback.get(&user).copied().unwrap_or_default(),
            })
            .collect();
        let video_bindings = reducer
            .video_routes()
            .values()
            .map(|route| VideoBinding {
                track_id: route.track_id.clone(),
                participant: route.participant,
                kind: route.kind,
                first_seq: route.first_seq,
                destination_id: self.visible_video_destination(route).map(|dest| dest.id()),
            })
            .collect();
        CallSnapshot {
            version: self.version,
            account,
            generation: self.generation,
            attempt: self.attempt,
            channel: self.channel,
            phase: self.phase,
            peer_id: self.peer_id.clone(),
            video_codec: self.video_codec,
            settings: self.settings,
            mic_capture: self.mic_capture,
            camera_capture: self.camera_capture,
            transport: self.transport,
            warnings: self.warnings.clone(),
            playback: self.playback.clone(),
            participants,
            audio_routes_known: reducer.audio_routes_known(),
            audio_routes: reducer.audio_routes().clone(),
            video_bindings,
            host_audio_focus: self.host_audio_focus,
        }
    }

    fn visible_video_destination(
        &self,
        route: &ReducedVideoRoute,
    ) -> Option<&Arc<dyn VideoDestination>> {
        route.fence?;
        let target = VideoTarget {
            participant: route.participant,
            kind: route.kind,
        };
        if !self.subscriptions.is_interested(target) {
            return None;
        }
        self.destinations.get(&(target.participant, target.kind))
    }

    /// Advances the monotonic per-attempt slot-plan revision and projects `SetVideoSlotPlan`.
    pub(crate) fn next_slot_plan(
        &mut self,
        reducer: &VoiceReducer,
    ) -> (u64, Vec<VideoSlotBinding>) {
        self.video_slot_plan_revision = self.video_slot_plan_revision.saturating_add(1);
        let slots = reducer
            .video_routes()
            .values()
            .map(|route| VideoSlotBinding {
                track_id: route.track_id.clone(),
                participant: route.participant,
                kind: route.kind,
                fence: route.fence,
                destination: self.visible_video_destination(route).cloned(),
            })
            .collect();
        (self.video_slot_plan_revision, slots)
    }

    /// Appends `warning` if not already present.
    pub(crate) fn push_warning(&mut self, warning: CallWarning) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    /// Removes `warning` if present.
    pub(crate) fn clear_warning(&mut self, warning: CallWarning) {
        self.warnings.retain(|existing| *existing != warning);
    }
}

/// Side effects collected while holding `CoordinatorState` and executed after releasing the lock.
///
/// Wire commands are enqueued into the non-blocking voice FIFO under `CoordinatorState` in exact
/// state-mutation order; active-attempt media commands are queued into `ActiveAttempt::pending_media_commands`
/// and drained serially outside `CoordinatorState` under `SessionCalls::media_dispatch`.
#[derive(Default)]
pub(crate) struct SideEffects {
    pub drain_media: bool,
    pub stop_leases: Vec<(CallAttemptId, Arc<dyn MediaLease>)>,
    pub revoke_destinations: Vec<Arc<dyn VideoDestination>>,
    pub schedule_retry_at: Option<(CallAttemptId, Instant, CancellationToken)>,
}

/// Mutable state guarded by `SessionCalls::state`.
pub(crate) struct CoordinatorState {
    pub generation: Generation,
    pub online: bool,
    pub resources: Arc<ResourceSnapshot>,
    pub reducer: VoiceReducer,
    pub rooms_version: u64,
    pub active: Option<ActiveAttempt>,
    pub draining_attempts: BTreeSet<CallAttemptId>,
    pub abandoned_joins: VecDeque<AbandonedJoin>,
    pub command_sender: Option<(Generation, VoiceCommandSender)>,
}
