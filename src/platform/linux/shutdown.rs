//! Linux session shutdown notification.
//!
//! A normal window close may intentionally hide Papo in the tray. During a
//! system shutdown/reboot that same policy is wrong: the compositor needs the
//! process to leave. Monitor the desktop portal (also available in Flatpak),
//! logind where accessible, and process termination signals independently.

use futures_util::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};

static WATCH_STARTED: AtomicBool = AtomicBool::new(false);
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn spawn(repaint: egui::Context) {
    if WATCH_STARTED.swap(true, Ordering::AcqRel) {
        return;
    }

    if let Err(error) = std::thread::Builder::new()
        .name("papo-shutdown".into())
        .spawn(move || {
            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    log::warn!("shutdown watcher sem runtime: {error}");
                    return;
                }
            };

            runtime.block_on(async move {
                tokio::join!(
                    async {
                        if let Err(error) = watch_portal(&repaint).await {
                            log::warn!("session portal watcher indisponível: {error}");
                        }
                    },
                    async {
                        if let Err(error) = watch_logind(&repaint).await {
                            log::debug!("logind watcher indisponível: {error}");
                        }
                    },
                    async {
                        if let Err(error) = watch_signals(&repaint).await {
                            log::warn!("termination watcher indisponível: {error}");
                        }
                    },
                );
            });
        })
    {
        WATCH_STARTED.store(false, Ordering::Release);
        log::warn!("shutdown watcher não iniciou: {error}");
    }
}

fn request_exit(repaint: &egui::Context) {
    SHUTDOWN_REQUESTED.store(true, Ordering::Release);
    // A hidden Wayland surface may never receive another frame callback.
    // Send the native close command as well as waking App::logic, which sees
    // the latch and therefore never cancels this close into the tray.
    repaint.send_viewport_cmd(egui::ViewportCommand::Close);
    repaint.request_repaint();
}

async fn watch_signals(repaint: &egui::Context) -> std::io::Result<()> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut terminate = signal(SignalKind::terminate())?;
    let mut interrupt = signal(SignalKind::interrupt())?;
    tokio::select! {
        _ = terminate.recv() => {},
        _ = interrupt.recv() => {},
    }
    log::info!("termination signal: saving and closing Papo");
    request_exit(repaint);
    Ok(())
}

async fn watch_portal(repaint: &egui::Context) -> anyhow::Result<()> {
    use ashpd::desktop::inhibit::{InhibitProxy, SessionState};
    let proxy = InhibitProxy::new().await?;
    // Subscribe before creating the monitor so its first state is not lost.
    let mut states = proxy.receive_state_changed().await?;
    let session = proxy.create_monitor(None).await?;
    // ashpd exposes the D-Bus session path through Serialize, not an accessor.
    let session_handle: String = serde_json::from_value(serde_json::to_value(&session)?)?;
    log::info!("session shutdown portal monitor active");
    while let Some(state) = states.next().await {
        if state.session_handle().as_str() != session_handle {
            continue;
        }
        match state.session_state() {
            SessionState::QueryEnd => {
                // Acknowledge promptly, without inhibiting logout. Another app
                // may still cancel it, so do not exit until Ending arrives.
                proxy.query_end_response(&session).await?;
            }
            SessionState::Ending => {
                request_exit(repaint);
                break;
            }
            SessionState::Running => {}
        }
    }
    Ok(())
}

async fn watch_logind(repaint: &egui::Context) -> zbus::Result<()> {
    let connection = zbus::Connection::system().await?;
    let proxy = zbus::Proxy::new(
        &connection,
        "org.freedesktop.login1",
        "/org/freedesktop/login1",
        "org.freedesktop.login1.Manager",
    )
    .await?;
    let mut signals = proxy.receive_signal("PrepareForShutdown").await?;

    while let Some(message) = signals.next().await {
        let (starting,): (bool,) = match message.body().deserialize() {
            Ok(body) => body,
            Err(error) => {
                log::warn!("sinal de desligamento inválido: {error}");
                continue;
            }
        };
        if starting {
            request_exit(repaint);
            break;
        }
    }
    Ok(())
}

pub fn requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::Acquire)
}
