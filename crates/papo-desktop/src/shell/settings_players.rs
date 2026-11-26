//! Settings › General › Embedded players from the shell (`docs/design.md` §15.7): the switch
//! (off releases every player; nothing is loaded from a site again until it is on), Remove
//! (applies at once; that site's next player asks again), and Clear data (everything the
//! site's player stored in Papo's profile goes). The page asks the WebView thread which sites
//! hold data when it opens and after each clear. The setting itself is `embedded_players`'.

use papo_sdk_ui::PlayerProvider;
use rune::Tx;

use super::Shell;
use crate::embedded_players::PlayerSettings;
use crate::settings::{RowId, Target};

impl Shell {
    /// Shows `settings` at once and saves them; a failure shows under `row`.
    pub(super) fn set_players(
        &mut self,
        settings: PlayerSettings,
        row: Option<RowId>,
        tx: &mut Tx,
    ) {
        self.latest_rows.players = row;
        if let Some(next) = self.players.settings.replace(settings) {
            self.write_players(next, row, tx);
        }
        self.players_follow_setting();
    }

    /// Writes `settings`; a failure takes the setting back to what the file holds and shows
    /// under `row`, if any.
    pub(super) fn write_players(
        &mut self,
        settings: PlayerSettings,
        row: Option<RowId>,
        tx: &mut Tx,
    ) {
        let write = self.players.settings.write_of(&settings);
        self.run_write(
            write,
            move |shell, ok, tx| {
                if !ok {
                    log::warn!("Embedded players was not saved");
                }
                let next = shell.players.settings.written(settings.clone(), ok);
                if let (Some(panel), Some(row)) = (shell.settings.as_mut(), row) {
                    panel.saved(row, Target::Players(settings), ok);
                }
                shell.players_follow_setting();
                if let Some(next) = next {
                    shell.write_players(next, shell.latest_rows.players, tx);
                }
            },
            tx,
        );
    }

    /// Players switched off: every player goes and every poster's button reads Open in
    /// browser.
    fn players_follow_setting(&mut self) {
        if !self.players.settings.settings().on {
            let commands = self.players.store.release_all();
            self.run_player_commands(commands);
        }
    }

    /// Play videos and music from other sites, flipped.
    pub(super) fn settings_players_on(&mut self, on: bool, tx: &mut Tx) {
        let mut next = self.players.settings.settings().clone();
        next.on = on;
        self.set_players(next, Some(RowId::PlayersOn), tx);
    }

    /// A site's Remove: it asks again from its next player.
    pub(crate) fn settings_players_remove(&mut self, provider: PlayerProvider, tx: &mut Tx) {
        let mut next = self.players.settings.settings().clone();
        next.allowed.remove(&provider);
        self.set_players(next, Some(RowId::PlayersSite(provider)), tx);
    }

    /// A site's Clear data: its live players go first, then everything it stored.
    pub(crate) fn settings_players_clear(&mut self, provider: PlayerProvider, _tx: &mut Tx) {
        let live: Vec<_> = self
            .players
            .store
            .live_pages()
            .into_iter()
            .filter(|(key, _)| self.player_provider(*key) == Some(provider))
            .map(|(key, _)| key)
            .collect();
        for key in live {
            let commands = self.players.store.close(key);
            self.run_player_commands(commands);
        }
        self.start_web_views();
        if let Some(views) = self.players.views() {
            views.clear_site(provider.key().to_owned());
        }
    }

    /// The Embedded players page opened: which sites hold data is asked again.
    pub(super) fn players_page_opened(&mut self) {
        self.start_web_views();
        if let Some(views) = self.players.views() {
            for provider in PlayerProvider::ALL {
                views.report_site(provider.key().to_owned());
            }
        }
    }
}
