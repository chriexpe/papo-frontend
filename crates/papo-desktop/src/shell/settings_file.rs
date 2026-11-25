//! Running writes of this device's settings file (`device_settings`) off
//! the UI thread: each is a job on the SDK client's runtime, which the
//! exit fence lets finish, and its owner hears back on the UI thread.
//!
//! Owns only where a write runs; what it writes and what a failure undoes
//! are its owner's.

use rune::Tx;

use super::Shell;
use crate::device_settings::Write;
use crate::settings::RowId;

/// The settings row behind each store's latest change, if a row asked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LatestRows {
    pub(crate) conversations: Option<RowId>,
    pub(crate) general: Option<RowId>,
    pub(crate) trusted: Option<RowId>,
}

impl Shell {
    /// Runs `write`, then `done` with whether it landed. Before the client
    /// opens (and in tests), here and now.
    pub(crate) fn run_write(
        &mut self,
        write: Write,
        done: impl FnOnce(&mut Shell, bool, &mut Tx) + Send + 'static,
        tx: &mut Tx,
    ) {
        let Some(sdk) = &self.sdk else {
            let ok = write.run().is_ok();
            done(self, ok, tx);
            return;
        };
        let updates = sdk.updates();
        sdk.finish(move |_client| async move {
            let ok = tokio::task::spawn_blocking(move || write.run().is_ok())
                .await
                .unwrap_or(false);
            updates.send_tx(move |shell, tx| {
                done(shell, ok, tx);
                true
            });
        });
    }

    /// The window closes: each change still waiting behind a running write
    /// is written now, as its write's callback will not run. Writes are
    /// numbered (`device_settings`), so one that lands after the running
    /// write is still the one the file keeps.
    pub(crate) fn flush_settings(&mut self) {
        let waiting = [
            self.appearance.pending_write(),
            self.conversations.pending_write(),
            self.general.pending_write(),
            self.content.trusted.pending_write(),
            self.layout.pending_write(),
            self.recent_jumps.pending_write(),
            self.videos.volume.pending_write(),
        ];
        for write in waiting.into_iter().flatten() {
            match &self.sdk {
                Some(sdk) => sdk.finish(move |_client| async move {
                    let _ = tokio::task::spawn_blocking(move || write.run()).await;
                }),
                None => {
                    let _ = write.run();
                }
            }
        }
    }

    /// Writes the remembered layout, and whatever change waited for it.
    pub(crate) fn write_layout(&mut self, write: Write, tx: &mut Tx) {
        self.run_write(
            write,
            |shell, ok, tx| {
                if let Some(next) = shell.layout.written(ok) {
                    shell.write_layout(next, tx);
                }
            },
            tx,
        );
    }

    /// Writes the videos' volume, and whatever change waited for it.
    pub(crate) fn write_video_volume(&mut self, write: Write, tx: &mut Tx) {
        self.run_write(
            write,
            |shell, ok, tx| {
                if let Some(next) = shell.videos.volume.written(ok) {
                    shell.write_video_volume(next, tx);
                }
            },
            tx,
        );
    }

    /// Writes the recent jumps, and whatever change waited for them.
    pub(crate) fn write_recent_jumps(&mut self, write: Write, tx: &mut Tx) {
        self.run_write(
            write,
            |shell, ok, tx| {
                if let Some(next) = shell.recent_jumps.written(ok) {
                    shell.write_recent_jumps(next, tx);
                }
            },
            tx,
        );
    }
}
