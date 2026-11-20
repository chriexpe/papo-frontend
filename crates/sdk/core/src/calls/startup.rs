//! Asynchronous call startup (`Preparing` ICE fetch + media start), join timeout and drain watchdogs.

use crate::{ClientError, ClientHandle, Generation, session::SessionInner};
use papo_api::gateway::Command;
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{
    coordinator::{SessionCalls, SideEffects},
    fifo::QueuedVoiceCommand,
    lease::ClientCallLease,
    media_port::{
        IceServerConfig, InitialMediaSettings, MEDIA_REPORT_CHANNEL_CAPACITY, MediaCommand,
        MediaEngineFactory, MediaReportSink, MediaStartError,
    },
    model::{CallAttemptId, CallEndReason, CallError, CallPhase},
};

/// Future returned by `IceFetcher::fetch`.
pub(crate) type IceFetchFuture =
    Pin<Box<dyn Future<Output = Result<Vec<IceServerConfig>, ClientError>> + Send>>;

/// Internal abstraction for fetching ICE servers; tests inject deterministic or gated fakes.
pub(crate) trait IceFetcher: Send + Sync + 'static {
    /// Fetches STUN/TURN servers for `generation`.
    fn fetch(&self, generation: Generation) -> IceFetchFuture;
}

/// Production `IceFetcher` reading `/voice/ice-servers` through the session's verified REST client.
pub(crate) struct SessionIceFetcher {
    pub client: ClientHandle,
    pub session: Arc<SessionInner>,
}

impl IceFetcher for SessionIceFetcher {
    fn fetch(&self, generation: Generation) -> IceFetchFuture {
        let client = self.client.clone();
        let session = Arc::clone(&self.session);
        Box::pin(async move {
            let rest = {
                let guard = session.verified_rest.lock().await;
                match guard.as_ref() {
                    Some((rest_gen, rest)) if *rest_gen == generation => rest.clone(),
                    _ => return Err(ClientError::Superseded),
                }
            };
            session.epochs.validate(generation)?;
            let response = client.inner.request(rest.ice_servers()).await?;
            session.epochs.validate(generation)?;
            Ok(response
                .ice_servers
                .unwrap_or_default()
                .into_iter()
                .map(IceServerConfig::from_wire)
                .collect())
        })
    }
}

/// Spawns the `Preparing` startup sequence: optional prior drain → ICE fetch → media start → `voice_join`.
pub(crate) fn spawn_prepare(
    calls: Arc<SessionCalls>,
    attempt: CallAttemptId,
    generation: Generation,
    wait_drain: Option<CallAttemptId>,
    cancel: CancellationToken,
    media_factory: Arc<dyn MediaEngineFactory>,
    ice_fetcher: Arc<dyn IceFetcher>,
    runtime: &tokio::runtime::Handle,
) {
    let rt_handle = runtime.clone();
    runtime.spawn(async move {
        if let Some(prev) = wait_drain
            && !await_prior_drain(&calls, attempt, prev, &cancel, &rt_handle).await
        {
            return;
        }
        let ice_result = tokio::select! {
            biased;
            () = cancel.cancelled() => return,
            result = ice_fetcher.fetch(generation) => result,
        };
        let Ok(ice_servers) = ice_result else {
            fail_prepare(&calls, attempt, CallEndReason::IceFetchFailed, &rt_handle);
            return;
        };
        let Some(initial_settings) = begin_media_start(&calls, attempt, generation, &cancel) else {
            return;
        };
        let (report_sink, mut report_rx) = MediaReportSink::channel(MEDIA_REPORT_CHANNEL_CAPACITY);
        let pump_calls = Arc::clone(&calls);
        let pump_rt = rt_handle.clone();
        rt_handle.spawn(async move {
            while let Some(report) = report_rx.recv().await {
                if report.attempt != attempt {
                    continue;
                }
                let is_drained = matches!(report.kind, super::media_port::MediaReportKind::Drained);
                pump_calls.on_media_report(report, &pump_rt);
                if is_drained {
                    break;
                }
            }
        });
        // Never abort mid-startup so a late completion after cancel can be explicitly stopped.
        let start_result = media_factory
            .start(attempt, ice_servers, initial_settings, report_sink)
            .await;
        finish_media_start(
            &calls,
            attempt,
            generation,
            initial_settings,
            &cancel,
            start_result,
            &rt_handle,
        );
    });
}

async fn await_prior_drain(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    prev: CallAttemptId,
    cancel: &CancellationToken,
    rt_handle: &tokio::runtime::Handle,
) -> bool {
    let drain_result = tokio::select! {
        biased;
        () = cancel.cancelled() => return false,
        result = tokio::time::timeout(
            calls.config.drain_timeout,
            calls.client_lease.wait_drained(prev),
        ) => result,
    };
    match drain_result {
        Ok(Ok(())) => true,
        Ok(Err(CallError::ReleasePending)) | Err(_) => {
            calls.client_lease.mark_release_pending(prev);
            fail_prepare(calls, attempt, CallEndReason::ReleasePending, rt_handle);
            false
        }
        Ok(Err(_)) => {
            fail_prepare(calls, attempt, CallEndReason::DeviceUnavailable, rt_handle);
            false
        }
    }
}

fn begin_media_start(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    generation: Generation,
    cancel: &CancellationToken,
) -> Option<InitialMediaSettings> {
    let mut guard = calls
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let active = guard.active.as_mut()?;
    if active.attempt != attempt
        || active.generation != generation
        || active.phase != CallPhase::Preparing
        || cancel.is_cancelled()
    {
        return None;
    }
    active.media_start_in_flight = true;
    Some(InitialMediaSettings {
        muted: active.settings.muted,
        camera_on: active.settings.camera_on,
        camera_facing: active.settings.camera_facing,
    })
}

fn finish_media_start(
    calls: &Arc<SessionCalls>,
    attempt: CallAttemptId,
    generation: Generation,
    initial_settings: InitialMediaSettings,
    cancel: &CancellationToken,
    start_result: Result<Box<dyn super::media_port::MediaLease>, MediaStartError>,
    rt_handle: &tokio::runtime::Handle,
) {
    match start_result {
        Err(error) => {
            {
                let mut guard = calls
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(active) = guard
                    .active
                    .as_mut()
                    .filter(|active| active.attempt == attempt)
                {
                    active.media_start_in_flight = false;
                }
                guard.draining_attempts.remove(&attempt);
            }
            let reason = match error {
                MediaStartError::UnsupportedCodec => CallEndReason::CodecUnsupported,
                MediaStartError::DeviceUnavailable
                | MediaStartError::Cancelled
                | MediaStartError::Internal => CallEndReason::DeviceUnavailable,
            };
            // End the attempt first (transitioning Active -> Free), then notify any waiters
            // in Draining/ReleasePending if the attempt was already cancelled during start().
            fail_prepare(calls, attempt, reason, rt_handle);
            calls.client_lease.mark_drained(attempt);
        }
        Ok(lease) => {
            let lease: Arc<dyn super::media_port::MediaLease> = Arc::from(lease);
            let mut effects = SideEffects::default();
            let mut join_cancel = None;
            {
                let mut guard = calls
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let still_current = guard.active.as_ref().is_some_and(|active| {
                    active.attempt == attempt
                        && active.generation == generation
                        && active.phase == CallPhase::Preparing
                        && !cancel.is_cancelled()
                });
                if still_current {
                    let join_cmd = if let Some(active) = guard.active.as_mut() {
                        active.media_start_in_flight = false;
                        active.media_lease = Some(lease);
                        active.phase = CallPhase::Joining;
                        active.join_state = super::attempt::JoinDispatchState::Queued;
                        join_cancel = Some(active.cancel.clone());
                        // Sync any controls invoked while start() was in flight.
                        sync_pre_start_controls(active, initial_settings, &mut effects);
                        Some(QueuedVoiceCommand {
                            generation,
                            attempt,
                            command: Command::VoiceJoin {
                                channel_id: active.channel.to_string(),
                            },
                            allow_after_end: false,
                        })
                    } else {
                        None
                    };
                    if let Some(cmd) = join_cmd {
                        if calls.enqueue_wire_locked(&mut guard, cmd, &mut effects) {
                            calls.publish_active_locked(&mut guard);
                        } else {
                            join_cancel = None;
                        }
                    }
                } else {
                    guard.draining_attempts.insert(attempt);
                    effects.stop_leases.push((attempt, lease));
                }
            }
            calls.execute_effects(effects, rt_handle);
            if let Some(join_cancel) = join_cancel {
                spawn_join_timeout(
                    Arc::clone(calls),
                    attempt,
                    calls.config.join_timeout,
                    join_cancel,
                    rt_handle,
                );
            }
        }
    }
}

fn sync_pre_start_controls(
    active: &mut super::attempt::ActiveAttempt,
    initial_settings: InitialMediaSettings,
    effects: &mut SideEffects,
) {
    if active.settings.muted != initial_settings.muted {
        active
            .pending_media_commands
            .push_back(MediaCommand::SetMuted {
                muted: active.settings.muted,
            });
        effects.drain_media = true;
    }
    if active.settings.camera_facing != initial_settings.camera_facing {
        active
            .pending_media_commands
            .push_back(MediaCommand::SetCameraFacing {
                facing: active.settings.camera_facing,
            });
        effects.drain_media = true;
    }
    if active.settings.camera_on != initial_settings.camera_on {
        active
            .pending_media_commands
            .push_back(MediaCommand::SetCamera {
                on: active.settings.camera_on,
            });
        effects.drain_media = true;
    }
    for (&user, &playback) in &active.playback {
        active
            .pending_media_commands
            .push_back(MediaCommand::SetPlayback { user, playback });
        effects.drain_media = true;
    }
}

fn fail_prepare(
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
            .is_some_and(|active| active.attempt == attempt && active.phase == CallPhase::Preparing)
        {
            calls.finish_active_locked(&mut guard, reason, false, &mut effects);
        }
    }
    calls.execute_effects(effects, runtime);
}

fn spawn_join_timeout(
    calls: Arc<SessionCalls>,
    attempt: CallAttemptId,
    timeout: Duration,
    cancel: CancellationToken,
    runtime: &tokio::runtime::Handle,
) {
    let deadline = Instant::now() + timeout;
    let rt_handle = runtime.clone();
    runtime.spawn(async move {
        tokio::select! {
            biased;
            () = cancel.cancelled() => return,
            () = tokio::time::sleep_until(deadline) => {}
        }
        calls.on_join_timeout(attempt, &rt_handle);
    });
}

/// Bounds a stopping media lease's drain so a stuck driver transitions cleanly to `ReleasePending`.
pub(crate) fn spawn_drain_watchdog(
    client_lease: Arc<ClientCallLease>,
    attempt: CallAttemptId,
    timeout: Duration,
    runtime: &tokio::runtime::Handle,
) {
    let deadline = Instant::now() + timeout;
    runtime.spawn(async move {
        if tokio::time::timeout_at(deadline, client_lease.wait_drained(attempt))
            .await
            .is_err()
        {
            client_lease.mark_release_pending(attempt);
        }
    });
}

/// Schedules a single monotonic wakeup for video subscription retry backoff or rate-limit pacing.
pub(crate) fn spawn_subscription_wakeup(
    calls: Arc<SessionCalls>,
    attempt: CallAttemptId,
    at: Instant,
    cancel: CancellationToken,
    runtime: &tokio::runtime::Handle,
) {
    let rt_handle = runtime.clone();
    runtime.spawn(async move {
        tokio::select! {
            biased;
            () = cancel.cancelled() => return,
            () = tokio::time::sleep_until(at) => {}
        }
        calls.on_subscription_wakeup(attempt, at, &rt_handle);
    });
}
