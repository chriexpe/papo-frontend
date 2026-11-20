//! Attempt-fenced media engine report dispatch for `SessionCalls`.
//! Turns local offers/answers/candidates into ordered voice FIFO commands and tracks transport/capture.

use papo_api::gateway::Command;
use std::sync::Arc;

use super::{
    coordinator::{CoordinatorState, SessionCalls, SideEffects},
    fifo::QueuedVoiceCommand,
    media_port::{
        MediaFatalError, MediaReport, MediaReportKind, MediaTransportState, MediaWarning,
    },
    model::{CallEndReason, CallPhase, CallWarning},
};

impl SessionCalls {
    /// Processes an attempt-tagged report emitted by the media engine.
    pub(crate) fn on_media_report(
        self: &Arc<Self>,
        report: MediaReport,
        runtime: &tokio::runtime::Handle,
    ) {
        if matches!(report.kind, MediaReportKind::Drained) {
            {
                let mut guard = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard.draining_attempts.remove(&report.attempt);
            }
            self.client_lease.mark_drained(report.attempt);
            return;
        }

        let mut effects = SideEffects::default();
        {
            let mut guard = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.apply_media_report_locked(&mut guard, report, &mut effects);
        }
        self.execute_effects(effects, runtime);
    }

    fn apply_media_report_locked(
        &self,
        guard: &mut CoordinatorState,
        report: MediaReport,
        effects: &mut SideEffects,
    ) {
        let Some(active) = guard.active.as_mut() else {
            return;
        };
        if active.attempt != report.attempt || !active.phase.is_active() {
            return;
        }
        let channel_str = active.channel.to_string();
        let attempt = active.attempt;
        let generation = active.generation;
        let joined = matches!(active.phase, CallPhase::Joined | CallPhase::Connected);
        match report.kind {
            MediaReportKind::LocalOffer { sdp } if joined => {
                effects.wire_commands.push(QueuedVoiceCommand {
                    generation,
                    attempt,
                    command: Command::VoiceOffer {
                        channel_id: channel_str,
                        sdp,
                    },
                    allow_after_end: false,
                });
            }
            MediaReportKind::LocalAnswer { sdp } if joined => {
                effects.wire_commands.push(QueuedVoiceCommand {
                    generation,
                    attempt,
                    command: Command::VoiceAnswer {
                        channel_id: channel_str,
                        sdp,
                    },
                    allow_after_end: false,
                });
            }
            MediaReportKind::LocalIceCandidate {
                candidate,
                sdp_mid,
                sdp_mline_index,
            } if joined => {
                effects.wire_commands.push(QueuedVoiceCommand {
                    generation,
                    attempt,
                    command: Command::VoiceIceCandidate {
                        channel_id: channel_str,
                        candidate,
                        sdp_mid,
                        sdp_mline_index,
                    },
                    allow_after_end: false,
                });
            }
            MediaReportKind::TransportState(state) => {
                self.apply_transport_state_locked(guard, state, effects);
            }
            MediaReportKind::MicCapture(state) => {
                active.mic_capture = state;
                self.stage_active_publish(guard, effects);
            }
            MediaReportKind::CameraCapture(state) => {
                active.camera_capture = state;
                self.stage_active_publish(guard, effects);
            }
            MediaReportKind::Warning(warning) => {
                active.push_warning(map_media_warning(warning));
                self.stage_active_publish(guard, effects);
            }
            MediaReportKind::Fatal(error) => {
                self.finish_active_locked(guard, map_media_fatal(error), true, effects);
            }
            MediaReportKind::LocalOffer { .. }
            | MediaReportKind::LocalAnswer { .. }
            | MediaReportKind::LocalIceCandidate { .. }
            | MediaReportKind::Drained => {}
        }
    }

    fn apply_transport_state_locked(
        &self,
        guard: &mut CoordinatorState,
        state: MediaTransportState,
        effects: &mut SideEffects,
    ) {
        let Some(active) = guard.active.as_mut() else {
            return;
        };
        active.transport = state;
        match state {
            MediaTransportState::Connected => {
                if active.phase == CallPhase::Joined {
                    active.phase = CallPhase::Connected;
                }
                active.clear_warning(CallWarning::IceDisconnected);
                self.stage_active_publish(guard, effects);
            }
            MediaTransportState::Disconnected => {
                active.push_warning(CallWarning::IceDisconnected);
                self.stage_active_publish(guard, effects);
            }
            MediaTransportState::Failed | MediaTransportState::Closed => {
                self.finish_active_locked(guard, CallEndReason::NegotiationFailed, true, effects);
            }
            MediaTransportState::New | MediaTransportState::Connecting => {
                self.stage_active_publish(guard, effects);
            }
        }
    }
}

const fn map_media_warning(warning: MediaWarning) -> CallWarning {
    match warning {
        MediaWarning::MicrophoneUnavailable => CallWarning::MicrophoneUnavailable,
        MediaWarning::CameraUnavailable => CallWarning::CameraUnavailable,
        MediaWarning::CodecFallback => CallWarning::CodecFallback,
        MediaWarning::IceDisconnected => CallWarning::IceDisconnected,
    }
}

const fn map_media_fatal(error: MediaFatalError) -> CallEndReason {
    match error {
        MediaFatalError::UnsupportedCodec => CallEndReason::CodecUnsupported,
        MediaFatalError::DeviceError => CallEndReason::DeviceUnavailable,
        MediaFatalError::NegotiationFailed
        | MediaFatalError::TransportFailed
        | MediaFatalError::Internal => CallEndReason::NegotiationFailed,
    }
}
