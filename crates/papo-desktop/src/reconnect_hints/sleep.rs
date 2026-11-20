//! Resume from suspend, from logind's `PrepareForSleep(false)` on the system
//! bus. A Flatpak needs `--system-talk-name=org.freedesktop.login1` for it.
//!
//! Owns only reading the signal. What a resume does is `reconnect_hints`'.

use futures_util::StreamExt;
use zbus::Connection;
use zbus::proxy::CacheProperties;

/// logind's manager, for its sleep signal only.
#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login {
    /// Sent with `start` true before the machine sleeps, false after it
    /// wakes.
    #[zbus(signal)]
    fn prepare_for_sleep(&self, start: bool) -> zbus::Result<()>;
}

/// The machine's resumes, as they happen.
pub(super) struct Resumes {
    signals: PrepareForSleepStream,
}

impl Resumes {
    /// Subscribes to logind on `bus`. Resumes after this returns are seen.
    ///
    /// # Errors
    ///
    /// The bus refused the subscription.
    pub(super) async fn subscribe(bus: &Connection) -> zbus::Result<Resumes> {
        let login = LoginProxy::builder(bus)
            .cache_properties(CacheProperties::No)
            .build()
            .await?;
        Ok(Resumes {
            signals: login.receive_prepare_for_sleep().await?,
        })
    }

    /// Waits for the next resume; `None` once the bus connection ends.
    pub(super) async fn next(&mut self) -> Option<()> {
        while let Some(signal) = self.signals.next().await {
            if signal.args().is_ok_and(|args| is_resume(args.start)) {
                return Some(());
            }
        }
        None
    }
}

/// Whether `PrepareForSleep(start)` says the machine woke. Going to sleep is
/// no hint: the sockets fail on their own, and nothing can reconnect yet.
pub(super) fn is_resume(start: bool) -> bool {
    !start
}
