//! The shell's side of the provider viewer (`docs/design.md` §15.5): the same live player
//! moves there (its page resized to the viewer's 16 : 9, playing on) and back to its row on
//! close, focus going to the player and back; Esc closes; the wheel goes to the page; Open in
//! browser and Copy link use the card's page, never the player's address. Drawing is
//! `viewer::provider`'s.

use rune::{Cx, El, Tx, WheelEvent};

use super::Shell;
use crate::design::Theme;
use crate::link_cards::player::page_key;
use crate::link_cards::player_of;
use crate::timeline::local_time::day_label;
use crate::timeline::{TimelineKey, author_name, created_us};
use crate::viewer::provider::{ProviderShown, view as provider_view};
use crate::web_view::PageInput;

impl Shell {
    /// The viewer takes player `key`'s live page; the image viewer closes first.
    pub(super) fn open_player_viewer(
        &mut self,
        key: crate::provider_players::PlayerKey,
        tx: &mut Tx,
    ) {
        let Some(page) = self.players.store.page(key) else {
            return;
        };
        if self.viewer.is_some() {
            self.close_viewer(tx);
        }
        self.nav.menu = None;
        self.players.store.set_viewer(Some(key));
        let size = self.player_viewer_box();
        if let Some(views) = self.players.views() {
            views.resize(page, size, self.scale);
        }
        tx.focus(page_key(key), true);
    }

    /// The viewer closes: the player goes back to its row, still playing, and keeps the
    /// keyboard there.
    pub(crate) fn close_player_viewer(&mut self, tx: &mut Tx) {
        let Some(key) = self.players.store.viewer() else {
            return;
        };
        self.players.store.set_viewer(None);
        let (size, _) = self.player_box(key);
        if let (Some(page), Some(views)) = (self.players.store.page(key), self.players.views()) {
            views.resize(page, size, self.scale);
        }
        tx.focus(page_key(key), true);
    }

    /// The player's box in the viewer: 16 : 9 at the largest in the room left between the
    /// pills (64 pt clear at the top, 24 at the sides and below).
    pub(super) fn player_viewer_box(&self) -> [f32; 2] {
        let [room_w, room_h] = self.expected_viewer_room();
        let aspect = papo_sdk_ui::PLAYER_ASPECT;
        let width = room_w.min(room_h * aspect).max(1.0);
        [width, (width / aspect).max(1.0)]
    }

    /// A key on the viewer around the player: Esc closes it.
    pub(crate) fn player_viewer_key(&mut self, event: &rune::edit::KeyEvent, tx: &mut Tx) -> bool {
        if event.key == rune::Key::Escape {
            self.close_player_viewer(tx);
            return true;
        }
        false
    }

    /// The wheel over the player in the viewer goes to the page (nothing scrolls behind it).
    pub(crate) fn player_viewer_wheel(&mut self, wheel: WheelEvent, tx: &mut Tx) -> bool {
        let Some(key) = self.players.store.viewer() else {
            return false;
        };
        if let (Some(page), Some(views)) = (self.players.store.page(key), self.players.views()) {
            // Rune moves content with the fingers; WPE scrolls by the wheel's turn.
            views.input(
                page,
                PageInput::Wheel {
                    x: wheel.x,
                    y: wheel.y,
                    dx: -wheel.dx,
                    dy: -wheel.dy,
                    precise: wheel.precise,
                },
            );
        }
        tx.live_only();
        true
    }

    /// Open in browser: the card's page, by §9's link rules.
    pub(crate) fn player_viewer_open_page(&mut self, tx: &mut Tx) {
        let Some(address) = self.player_viewer_page() else {
            return;
        };
        let Some(at) = self
            .players
            .store
            .viewer()
            .and_then(|key| self.players.ats.get(&key).copied())
        else {
            return;
        };
        self.close_player_viewer(tx);
        let timeline = TimelineKey {
            account: at.account,
            channel: at.channel,
        };
        self.link_pressed(timeline, address, tx);
    }

    /// Copy link: the card's page, never the player's address.
    pub(crate) fn player_viewer_copy_link(&mut self, tx: &mut Tx) {
        if let Some(address) = self.player_viewer_page() {
            tx.copy(address);
        }
    }

    fn player_viewer_page(&self) -> Option<String> {
        let key = self.players.store.viewer()?;
        let at = *self.players.ats.get(&key)?;
        Some(self.preview_at(at)?.page()?.address.clone())
    }
}

/// The provider viewer, while open; dark in every theme as the image viewer's pills are.
pub(super) fn view(shell: &Shell, cx: &mut Cx<Shell>) -> Option<El> {
    let key = shell.players.store.viewer()?;
    let at = *shell.players.ats.get(&key)?;
    let surface = shell.players.surface(key)?;
    let preview = shell.preview_at(at)?;
    let (_, descriptor) = player_of(at, &preview)?;
    let papo_sdk_ui::LinkPreview::Card(card) = &preview else {
        return None;
    };
    let message = shell.message_at(at)?;
    let resources = shell.tabs.shown().and_then(|open| open.resources.clone());
    let sender = author_name(&message, resources.as_deref());
    let when = created_us(&message).map_or_else(String::new, |at_us| {
        let day = shell.local_time.day_of(at_us);
        crate::strings::date_time(
            &day_label(day, shell.today),
            &shell.local_time.time_label(at_us),
        )
    });
    let system_dark = cx.system_dark();
    let mut appearance = shell.shown_appearance();
    if !appearance.theme_or(system_dark).is_dark() {
        appearance.theme = Some(Theme::Graphite);
    }
    let look = super::appearance::look_of(
        &appearance,
        shell.conversations.settings().mention_hue_degrees,
        system_dark,
        shell.system_font,
    );
    let shown = ProviderShown {
        at,
        key,
        surface: &surface,
        cursor: shell.players.cursor(key),
        size: shell.player_viewer_box(),
        provider: descriptor.provider.name(),
        title: card.title.as_deref(),
        sender: &sender,
        when: &when,
    };
    Some(provider_view(&shown, &look))
}
