//! The shell's side of Settings › Diagnostics (`docs/design.md` §8): the
//! facts the page shows (the panel's server and its account's connection,
//! the media cache measured when the page opens), Reconnect, and Clear
//! media cache, which asks inside its row first.
//!
//! Owns these transitions; the measurements and the jobs are
//! `diagnostics`', the connection the live watch's (`live_watch`).

use rune::Tx;

use super::Shell;
use crate::settings::RowId;
use crate::settings::rows::DiagnosticsFacts;

impl Shell {
    /// The server the panel is about and its shown account: the selected
    /// server's, else the first server's with one.
    pub(crate) fn settings_subject(
        &self,
    ) -> Option<(
        &crate::server_list::ServerEntry,
        &crate::server_list::AccountEntry,
    )> {
        let servers = self.nav.servers();
        let in_order = servers.order().into_iter().filter_map(|id| servers.get(id));
        self.nav
            .selected()
            .into_iter()
            .chain(in_order)
            .find_map(|server| Some((server, server.account.as_ref()?)))
    }

    /// What Diagnostics shows now.
    pub(crate) fn diagnostics_facts(&self) -> DiagnosticsFacts {
        let panel = self.settings.as_ref();
        let subject = self.settings_subject();
        DiagnosticsFacts {
            server: subject.map(|(server, _)| server.name.clone()),
            link: subject
                .and_then(|(_, account)| self.activity.get(&account.id))
                .and_then(|activity| activity.link),
            cache_bytes: panel.and_then(|panel| panel.cache_bytes),
            clearing: panel.is_some_and(|panel| panel.clearing),
            asking: panel.and_then(|panel| panel.asking),
        }
    }

    /// Diagnostics opened: the media cache is measured.
    pub(crate) fn measure_media_cache(&mut self) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        let ticket = self.diagnostics_requests.issue();
        sdk.request(
            ticket,
            |shell: &Shell| &shell.diagnostics_requests,
            crate::diagnostics::media_bytes,
            |shell: &mut Shell, bytes| {
                let Some(panel) = shell.settings.as_mut() else {
                    return false;
                };
                panel.cache_bytes = bytes;
                true
            },
        );
    }

    /// A destructive row pressed: it asks its question.
    pub(crate) fn settings_ask(&mut self, row: RowId, tx: &mut Tx) {
        if let Some(panel) = self.settings.as_mut() {
            panel.asking = Some(row);
            tx.focus(format!("{}-cancel", row.key()), true);
        }
    }

    /// A destructive row's Cancel.
    pub(crate) fn settings_cancel_ask(&mut self) {
        if let Some(panel) = self.settings.as_mut() {
            panel.asking = None;
        }
    }

    /// An action row, or a destructive row's confirmation.
    pub(crate) fn settings_action(&mut self, row: RowId, tx: &mut Tx) {
        match row {
            RowId::Reconnect => {
                let (Some(sdk), Some((_, account))) = (self.sdk.clone(), self.settings_subject())
                else {
                    return;
                };
                let account = account.id;
                sdk.run(
                    move |client| async move { crate::diagnostics::reconnect(&client, account) },
                );
            }
            RowId::ClearCache => self.clear_media_cache(tx),
            RowId::SignOut => self.sign_out_subject(),
            _ => {}
        }
    }

    /// Clears the media cache, then shows what it measures afterwards.
    fn clear_media_cache(&mut self, tx: &mut Tx) {
        let (Some(sdk), Some(panel)) = (self.sdk.clone(), self.settings.as_mut()) else {
            return;
        };
        panel.asking = None;
        panel.clearing = true;
        tx.focus(RowId::ClearCache.key(), true);
        let ticket = self.diagnostics_requests.issue();
        sdk.request(
            ticket,
            |shell: &Shell| &shell.diagnostics_requests,
            crate::diagnostics::clear_media,
            |shell: &mut Shell, (_cleared, bytes)| {
                let Some(panel) = shell.settings.as_mut() else {
                    return false;
                };
                panel.clearing = false;
                panel.cache_bytes = bytes;
                true
            },
        );
    }
}
