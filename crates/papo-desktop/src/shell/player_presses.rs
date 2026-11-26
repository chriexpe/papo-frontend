//! What a provider player's box, its menu and the keys ask for (`docs/design.md` §15.3,
//! §15.5, §15.6): play (asking first for a site not allowed, nothing at all with players off),
//! the ask's answers, Close player, Retry, Open in browser, the page's input and focus, the
//! card's menu, Space or K (any player) and O (a video) on a focused message, and the provider viewer taking the
//! player and giving it back. The players' state is `players`'.

use papo_sdk::Uuid;
use papo_sdk_ui::{LinkPreview, PlayerKind};
use rune::edit::KeyEvent;
use rune::{Key, Tx};

use super::Shell;
use crate::link_cards::player::{Consent, PlayerEvent, ask_play_key, page_key};
use crate::link_cards::{is_provider_video, player_of, plays_on_provider, previews};
use crate::message_media::received::{MediaAt, MediaSurface};
use crate::provider_players::{Phase, PlayerKey};
use crate::timeline::TimelineKey;
use crate::timeline::rows::message_part_key;
use crate::web_view::PageInput;
use crate::web_view::keys::key_press;

impl Shell {
    /// Something to do for player `key`, whose box is at `at`.
    pub(crate) fn player_event(
        &mut self,
        at: MediaAt,
        key: PlayerKey,
        event: PlayerEvent,
        tx: &mut Tx,
    ) {
        match event {
            PlayerEvent::Play => self.play_player(at, tx),
            PlayerEvent::Consent(consent) => self.consent(at, key, consent, tx),
            PlayerEvent::Close => self.close_player(at, key, tx),
            PlayerEvent::OpenInBrowser => self.open_player_page(at, tx),
            PlayerEvent::Retry => {
                self.players.ats.insert(key, at);
                let commands = self.players.store.restart(key, false, tx.clock());
                self.run_player_commands(commands);
            }
            PlayerEvent::Input(input) => {
                if matches!(input, PageInput::Down { .. }) {
                    self.player_used(key);
                }
                if let (Some(page), Some(views)) =
                    (self.players.store.page(key), self.players.views())
                {
                    views.input(page, input);
                }
                tx.live_only();
            }
            PlayerEvent::Focus(focused) => {
                if focused {
                    self.players.focused = Some(key);
                } else if self.players.focused == Some(key) {
                    self.players.focused = None;
                }
                if let (Some(page), Some(views)) =
                    (self.players.store.page(key), self.players.views())
                {
                    views.focus(page, focused);
                }
            }
            PlayerEvent::Hover(inside) => {
                let changed = if inside {
                    self.players.hovered.insert(key)
                } else {
                    self.players.hovered.remove(&key)
                };
                if !changed {
                    tx.live_only();
                }
            }
            PlayerEvent::Menu(point) => self.open_player_menu(at, key, Some(point)),
        }
    }

    /// A key while player `key`'s page has the keyboard: the first Esc returns to the message
    /// and the page never sees it (in the viewer, Esc closes the viewer); the Menu key opens
    /// the card's menu; a Ctrl (⌘) chord stays Papo's; every other key goes to the page.
    /// Returns whether it was taken.
    pub(crate) fn player_key(
        &mut self,
        at: MediaAt,
        key: PlayerKey,
        event: &KeyEvent,
        tx: &mut Tx,
    ) -> bool {
        match &event.key {
            Key::Escape if self.players.store.viewer() == Some(key) => {
                self.close_player_viewer(tx);
                true
            }
            Key::Escape => {
                tx.focus(message_part_key(at.message), true);
                true
            }
            Key::ContextMenu => {
                self.open_player_menu(at, key, None);
                true
            }
            Key::Command(_) => false,
            key_pressed => {
                let Some(press) = key_press(key_pressed, event.mods) else {
                    return false;
                };
                self.player_used(key);
                if let (Some(page), Some(views)) =
                    (self.players.store.page(key), self.players.views())
                {
                    views.input(page, PageInput::Key(press));
                }
                tx.live_only();
                true
            }
        }
    }

    /// A press on play at `at`: with players off or WPE missing, the page opens in the
    /// browser; a site not allowed asks first; a live player takes the keyboard.
    pub(super) fn play_player(&mut self, at: MediaAt, tx: &mut Tx) {
        let Some(preview) = self.preview_at(at) else {
            return;
        };
        let Some((key, descriptor)) = player_of(at, &preview) else {
            self.open_player_page(at, tx);
            return;
        };
        if !self.players.loadable() {
            self.open_player_page(at, tx);
            return;
        }
        self.want_players();
        self.players.ats.insert(key, at);
        match self.players.store.phase(key) {
            // Its page takes the keyboard once it draws.
            Some(Phase::Loading { .. }) => {}
            Some(Phase::Playing) => tx.focus(page_key(key), true),
            Some(Phase::Asking) => tx.focus(ask_play_key(key), true),
            None | Some(Phase::Stopped | Phase::Failed) => {
                if self.players.settings.allows(descriptor.provider) {
                    let commands =
                        self.players
                            .store
                            .start(key, descriptor.clone(), false, tx.clock());
                    self.run_player_commands(commands);
                } else {
                    self.players.store.ask(key, descriptor.clone());
                    tx.focus(ask_play_key(key), true);
                }
            }
        }
    }

    /// The ask's answer: Play once, Always allow the site then play, or Cancel (the poster
    /// returns and the message has the keyboard).
    fn consent(&mut self, at: MediaAt, key: PlayerKey, consent: Consent, tx: &mut Tx) {
        let Some(preview) = self.preview_at(at) else {
            return;
        };
        let Some((_, descriptor)) = player_of(at, &preview) else {
            return;
        };
        match consent {
            Consent::Cancel => {
                let commands = self.players.store.close(key);
                self.run_player_commands(commands);
                tx.focus(message_part_key(at.message), true);
            }
            Consent::Once | Consent::Always => {
                if consent == Consent::Always
                    && let Some(settings) = self.players.settings.allow(descriptor.provider)
                {
                    self.write_players(settings, None, tx);
                }
                let commands = self
                    .players
                    .store
                    .start(key, descriptor.clone(), false, tx.clock());
                self.run_player_commands(commands);
                tx.focus(message_part_key(at.message), true);
            }
        }
    }

    /// Close player: the poster returns, and the message has the keyboard.
    pub(super) fn close_player(&mut self, at: MediaAt, key: PlayerKey, tx: &mut Tx) {
        let commands = self.players.store.close(key);
        self.run_player_commands(commands);
        tx.focus(message_part_key(at.message), true);
    }

    /// Opens the card's page (never the player's address) in the browser, by §9's link rules.
    fn open_player_page(&mut self, at: MediaAt, tx: &mut Tx) {
        let Some(page) = self
            .preview_at(at)
            .and_then(|preview| preview.page().cloned())
        else {
            return;
        };
        let timeline = TimelineKey {
            account: at.account,
            channel: at.channel,
        };
        self.link_pressed(timeline, page.address, tx);
    }

    /// The person used player `key`: it goes last in line for release, and a quieted one
    /// sounds again, quieting other media.
    fn player_used(&mut self, key: PlayerKey) {
        let change = self.players.store.unquiet(key);
        if change.quiet_others {
            self.quiet_other_media();
        }
        self.run_player_commands(change.commands);
    }

    /// A key (`typed`) on focused message `id` (§15.4, §15.6). Space or K: its first provider
    /// player (video or track) plays, asking first when its site isn't allowed; a live one takes
    /// the keyboard; a provider off the allowlist opens in the browser. O: its first provider
    /// video opens in the viewer (a track has none). Returns whether a player took the key.
    pub(super) fn message_player_key(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        typed: &str,
        tx: &mut Tx,
    ) -> bool {
        if typed == " " || typed.eq_ignore_ascii_case("k") {
            let Some(at) = self.first_provider_player(key, id) else {
                return false;
            };
            self.play_player(at, tx);
            return true;
        }
        if typed.eq_ignore_ascii_case("o") {
            let Some(at) = self.first_provider_place(key, id, is_provider_video) else {
                return false;
            };
            let plays_here = self.preview_at(at).as_ref().is_some_and(|preview| {
                player_of(at, preview).is_some_and(|(_, player)| player.kind() == PlayerKind::Video)
            });
            if plays_here {
                self.player_viewer_from(at, tx);
                return true;
            }
        }
        false
    }

    /// The media place of message `id`'s first provider player (a video or a track), in the
    /// shown conversation.
    pub(super) fn first_provider_player(&self, key: TimelineKey, id: Uuid) -> Option<MediaAt> {
        self.first_provider_place(key, id, plays_on_provider)
    }

    /// The media place of message `id`'s first preview that `wanted` picks, in the shown
    /// conversation.
    fn first_provider_place(
        &self,
        key: TimelineKey,
        id: Uuid,
        wanted: fn(&LinkPreview) -> bool,
    ) -> Option<MediaAt> {
        let shown = self.conversations.settings().link_previews;
        let open = self.tabs.shown().filter(|open| open.key == key)?;
        let (message, _) = open.message(id)?;
        let place = previews(message, shown).iter().position(wanted)?;
        Some(MediaAt {
            surface: MediaSurface::Timeline,
            account: key.account,
            channel: key.channel,
            message: id,
            index: crate::link_cards::attachment_count(message) + place,
        })
    }

    /// O, or Open viewer: the viewer takes the player at `at`, loading it first (a site not
    /// allowed asks first, in the card); with players off the page opens in the browser.
    pub(super) fn player_viewer_from(&mut self, at: MediaAt, tx: &mut Tx) {
        let Some(preview) = self.preview_at(at) else {
            return;
        };
        let Some((key, _)) = player_of(at, &preview) else {
            self.open_player_page(at, tx);
            return;
        };
        if self.players.store.page(key).is_none() {
            self.play_player(at, tx);
        }
        if self.players.store.page(key).is_some() {
            self.open_player_viewer(key, tx);
        }
    }
}
