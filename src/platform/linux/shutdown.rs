//! Linux session shutdown notification.
//!
//! A normal window close may intentionally hide Papo in the tray. During a
//! system shutdown/reboot that same policy is wrong: the compositor needs the
//! process to leave. systemd-logind exposes that distinction on the system bus.

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

            let result: zbus::Result<()> = runtime.block_on(async move {
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
                        SHUTDOWN_REQUESTED.store(true, Ordering::Release);
                        repaint.request_repaint();
                        break;
                    }
                }
                Ok(())
            });

            if let Err(error) = result {
                // Session shutdown still works normally when logind is not
                // available; only close-to-tray cannot be disambiguated.
                log::warn!("shutdown watcher indisponível: {error}");
            }
        })
    {
        WATCH_STARTED.store(false, Ordering::Release);
        log::warn!("shutdown watcher não iniciou: {error}");
    }
}

pub fn requested() -> bool {
    SHUTDOWN_REQUESTED.load(Ordering::Acquire)
}
