//! The shell's side of the General settings (`docs/design.md` §8): the
//! language put into effect at once, the whole window redrawn in it and
//! the message being read kept in its place; Confirm external links; and
//! Trusted domains' add and Remove. Each saves off the UI thread, and a
//! failed save takes the setting back, the language too, and shows under
//! its row.
//!
//! Owns these transitions; the values and their storage are
//! `general_settings`' and `trusted_domains`'.

use std::collections::BTreeSet;

use rune::{TextEdit, Tx};

use super::Shell;
use crate::general_settings::GeneralSettings;
use crate::settings::{Change, RowId, Target};
use crate::trusted_domains::typed_domain;

impl Shell {
    /// Shows `settings` and saves them; `row` is the settings row that
    /// asked, if any.
    pub(crate) fn set_general(
        &mut self,
        settings: GeneralSettings,
        row: Option<RowId>,
        tx: &mut Tx,
    ) {
        let reading = self.reading_place();
        self.latest_rows.general = row;
        if let Some(next) = self.general.change(settings) {
            self.write_general(next, row, tx);
        }
        self.language_in_effect(tx);
        super::appearance::keep_place(reading, tx);
    }

    /// The language the settings hold, in effect: a new one redraws every
    /// view (the look's key holds the locale) and names the window again.
    fn language_in_effect(&mut self, tx: &mut Tx) {
        if crate::strings::use_language(self.general.settings().language) {
            tx.set_title(crate::strings::window_title());
        }
    }

    fn write_general(&mut self, settings: GeneralSettings, row: Option<RowId>, tx: &mut Tx) {
        let write = self.general.write_of(&settings);
        self.run_write(
            write,
            move |shell, ok, tx| shell.general_written(settings, row, ok, tx),
            tx,
        );
    }

    /// The write of `settings` landed, or did not: a failure takes them
    /// back, the language with them, and shows under the row that asked.
    fn general_written(
        &mut self,
        settings: GeneralSettings,
        row: Option<RowId>,
        ok: bool,
        tx: &mut Tx,
    ) {
        let reading = (!ok).then(|| self.reading_place()).flatten();
        if !ok {
            log::warn!("a General setting was not saved");
        }
        let next = self.general.written(settings, ok);
        if let (Some(panel), Some(row)) = (self.settings.as_mut(), row) {
            panel.saved(row, Target::General(settings), ok);
        }
        if !ok {
            self.language_in_effect(tx);
        }
        super::appearance::keep_place(reading, tx);
        if let Some(next) = next {
            // The change that waited is the latest one: its row hears.
            self.write_general(next, self.latest_rows.general, tx);
        }
    }

    /// Trusted domains' field, for its editor.
    pub(crate) fn settings_domain_mut(&mut self) -> &mut TextEdit {
        let Shell {
            settings,
            search_spare,
            ..
        } = self;
        settings.as_mut().map_or_else(
            || {
                *search_spare = TextEdit::new("");
                search_spare
            },
            |panel| &mut panel.domain,
        )
    }

    /// Enter in Trusted domains' field, or its Add: what was typed is
    /// trusted, or, naming no site, stays with its danger line.
    pub(crate) fn settings_trust_typed(&mut self, tx: &mut Tx) {
        let Some(panel) = self.settings.as_mut() else {
            return;
        };
        let Some(host) = typed_domain(&panel.domain.text) else {
            panel.domain_refused = true;
            return;
        };
        panel.domain = TextEdit::new("");
        panel.domain_refused = false;
        self.latest_rows.trusted = Some(RowId::TrustedAdd);
        if let Some(hosts) = self.content.trusted.add(host) {
            self.write_trusted(hosts, Some(RowId::TrustedAdd), tx);
        }
    }

    /// A trusted domain's Remove: it applies at once (§8: undone the same
    /// way, so nothing asks).
    pub(crate) fn settings_untrust(&mut self, host: &str, tx: &mut Tx) {
        let row = RowId::TrustedHost(crate::trusted_domains::host_id(host));
        self.latest_rows.trusted = Some(row);
        if let Some(hosts) = self.content.trusted.remove(host) {
            self.write_trusted(hosts, Some(row), tx);
        }
    }

    /// Writes the trusted domains; a failure takes the list back to what
    /// the file holds and shows under `row`, the row that asked, if any.
    pub(crate) fn write_trusted(
        &mut self,
        hosts: BTreeSet<String>,
        row: Option<RowId>,
        tx: &mut Tx,
    ) {
        let write = self.content.trusted.write_of(&hosts);
        self.run_write(
            write,
            move |shell, ok, tx| {
                let next = shell.content.trusted.written(hosts.clone(), ok);
                if let (Some(panel), Some(row)) = (shell.settings.as_mut(), row) {
                    panel.saved(row, Target::Trusted(hosts), ok);
                }
                if let Some(next) = next {
                    // The change that waited is the latest one: its row hears.
                    shell.write_trusted(next, shell.latest_rows.trusted, tx);
                }
            },
            tx,
        );
    }
}

/// `current` with `change` applied, when `change` is a General row's.
pub(super) fn applied(current: &GeneralSettings, change: &Change) -> Option<GeneralSettings> {
    let mut next = *current;
    match *change {
        Change::Language(language) => next.language = language,
        Change::Switch(RowId::ConfirmLinks, on) => next.confirm_links = on,
        _ => return None,
    }
    Some(next)
}
