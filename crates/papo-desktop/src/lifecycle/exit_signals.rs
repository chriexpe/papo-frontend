//! SIGTERM and SIGINT, the way a session logout, `kill` and Ctrl+C end the
//! process. Rune at the pin installs no handler, so without this the default
//! action ends the process in the middle of whatever the SDK is writing.
//!
//! Owns only waiting for the signals. What a signal does (quit the app, so
//! the window closes and the exit fence runs) is the lifecycle thread's.

use tokio::signal::unix::{SignalKind, signal};

/// The signals that end Papo, installed. From now on they no longer end the
/// process by themselves.
pub(super) struct ExitSignals {
    terminate: tokio::signal::unix::Signal,
    interrupt: tokio::signal::unix::Signal,
}

impl ExitSignals {
    /// Installs the handlers.
    ///
    /// # Errors
    ///
    /// The OS refused a handler; that signal keeps its default action.
    pub(super) fn install() -> std::io::Result<ExitSignals> {
        Ok(ExitSignals {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
        })
    }

    /// Waits for the next SIGTERM or SIGINT and returns its name, for the log.
    pub(super) async fn next(&mut self) -> &'static str {
        tokio::select! {
            _ = self.terminate.recv() => "SIGTERM",
            _ = self.interrupt.recv() => "SIGINT",
        }
    }
}
