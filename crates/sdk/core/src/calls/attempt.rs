//! Active call attempt record, snapshot/slot-plan projection and deferred coordinator side effects.
//! Coordinator transitions and execution live in `coordinator.rs` and `lifecycle.rs`.

use crate::{AccountId, Generation, ResourceSnapshot, Uuid, observable::StateCell};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{
    fifo::{QueuedVoiceCommand, VoiceCommandSender},
    media_port::{
        CaptureState, MediaCommand, MediaLease, MediaTransportState, VideoDestination,
        VideoSlotBinding,
    },
    model::{
        CallAttemptId, CallParticipant, CallPhase, CallSettings, CallSnapshot, CallWarning,
        HostLeaseKind, PlaybackSettings, RoomsSnapshot, VideoBinding, VideoCodec, VideoKind,
    },
    reducer::VoiceReducer,
    subscriptions::VideoSubscriptionPlanner,
};

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
    pub media_lease: Option<Box<dyn MediaLease>>,
    pub media_start_in_flight: bool,
    pub voice_join_sent: bool,
    pub host_audio_focus: bool,
    pub host_leases: BTreeMap<HostLeaseKind, u64>,
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
            media_lease: None,
            media_start_in_flight: false,
            voice_join_sent: false,
            host_audio_focus: true,
            host_leases: BTreeMap::new(),
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
                destination_id: self
                    .destinations
                    .get(&(route.participant, route.kind))
                    .map(|dest| dest.id()),
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

    /// Projects the current video slot plan for `MediaCommand::SetVideoSlotPlan`.
    pub(crate) fn slot_plan(&self, reducer: &VoiceReducer) -> (u64, Vec<VideoSlotBinding>) {
        let slots = reducer
            .video_routes()
            .values()
            .map(|route| VideoSlotBinding {
                track_id: route.track_id.clone(),
                participant: route.participant,
                kind: route.kind,
                first_seq: route.first_seq,
                destination: self
                    .destinations
                    .get(&(route.participant, route.kind))
                    .cloned(),
            })
            .collect();
        (reducer.video_revision(), slots)
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

/// Update to apply to `SessionCalls::active_cell` after releasing the coordinator lock.
#[derive(Default)]
pub(crate) enum ActivePublish {
    /// Leave `active_cell` untouched.
    #[default]
    Unchanged,
    /// Clear `active_cell` to `None`.
    Clear,
    /// Replace `active_cell` with `Some(snapshot)`.
    Set(Box<CallSnapshot>),
}

/// Side effects collected while holding `CoordinatorState` and executed after releasing the lock.
#[derive(Default)]
pub(crate) struct SideEffects {
    pub media_commands: Vec<MediaCommand>,
    pub stop_leases: Vec<(CallAttemptId, Box<dyn MediaLease>)>,
    pub revoke_destinations: Vec<Arc<dyn VideoDestination>>,
    pub wire_commands: Vec<QueuedVoiceCommand>,
    pub schedule_retry_at: Option<(CallAttemptId, Instant)>,
    pub closed_cells: Vec<(Arc<StateCell<CallSnapshot>>, CallSnapshot)>,
    pub active_publish: ActivePublish,
    pub rooms_publish: Option<RoomsSnapshot>,
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
    pub abandoned_joins: BTreeMap<Uuid, u32>,
    pub command_sender: Option<(Generation, VoiceCommandSender)>,
    pub next_host_lease_id: u64,
}
