//! Running writes of this device's settings file (`device_settings`) off
//! the UI thread: each is a job on the SDK client's runtime, which the
//! exit fence lets finish, and its owner hears back on the UI thread.
//!
//! Owns only where a write runs; what it writes and what a failure undoes
//! are its owner's.

use std::collections::BTreeSet;

use rune::Tx;

use super::Shell;
use crate::device_settings::Write;

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

    /// Writes the trusted domains; a failure takes the list back to what the
    /// file holds.
    pub(crate) fn write_trusted(&mut self, hosts: BTreeSet<String>, tx: &mut Tx) {
        let write = self.content.trusted.write_of(&hosts);
        self.run_write(
            write,
            move |shell, ok, tx| {
                if let Some(next) = shell.content.trusted.written(hosts, ok) {
                    shell.write_trusted(next, tx);
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
