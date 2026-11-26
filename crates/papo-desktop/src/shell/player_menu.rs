//! A provider video's menus (`docs/design.md` §15.6): its card's secondary click, Play (Space)
//! and Open viewer (O) at rest, Open viewer and Close player once live, then Open in browser and
//! Copy link with the page's host (never the player's address); and the same player items in
//! its message's menu, so the keyboard reaches them.

use papo_sdk::Uuid;
use rune::Tx;

use super::Shell;
use crate::icons;
use crate::link_cards::player_of;
use crate::links;
use crate::menu::{Entry, MenuAction, MenuItem, OpenMenu, Placement, PlayerAction};
use crate::message_media::received::MediaAt;
use crate::provider_players::PlayerKey;
use crate::strings;
use crate::timeline::TimelineKey;
use crate::timeline::rows::message_part_key;

impl Shell {
    /// The card's menu (§15.6): Play (Space) at rest, Open viewer (O) and Close player once
    /// live, then Open in browser and Copy link with the page's host. Beside the pointer, or
    /// (from the keyboard) beside the box.
    pub(super) fn open_player_menu(
        &mut self,
        at: MediaAt,
        key: PlayerKey,
        point: Option<[f32; 2]>,
    ) {
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
        let mut items = self.player_items(at, key);
        items.push(
            MenuItem::plain(
                "menu-player-browser",
                strings::embed_open_browser(),
                MenuAction::OpenLink {
                    key: timeline,
                    url: page.address.clone(),
                },
                false,
            )
            .with_icon(&icons::EXTERNAL)
            .with_sub(links::host(&page.address)),
        );
        items.push(
            MenuItem::plain(
                "menu-player-copy",
                strings::link_menu_copy(),
                MenuAction::CopyLink(page.address),
                false,
            )
            .with_icon(&icons::COPY),
        );
        let placement = match point {
            Some(point) => Placement::AtPointer(point),
            None => Placement::Beside(crate::link_cards::view::card_key(at)),
        };
        self.nav.close_accounts();
        self.nav.menu = Some(OpenMenu::with_entries(
            placement,
            message_part_key(at.message),
            items.into_iter().map(Entry::Item).collect(),
            crate::design::MESSAGE_MENU_WIDTH_PT,
        ));
    }

    /// A provider video's own menu items (§15.6): Play (Space) and Open viewer (O) at rest,
    /// Open viewer and Close player once live; none while players can't load.
    fn player_items(&self, at: MediaAt, key: PlayerKey) -> Vec<MenuItem> {
        let action = |action| MenuAction::Player { at, key, action };
        let viewer = MenuItem::plain(
            "menu-player-viewer",
            strings::embed_open_viewer(),
            action(PlayerAction::OpenViewer),
            false,
        )
        .with_icon(&icons::FULLSCREEN)
        .with_shortcut("O");
        if self.players.store.page(key).is_some() {
            let close = MenuItem::plain(
                "menu-player-close",
                strings::embed_close_player(),
                action(PlayerAction::Close),
                false,
            )
            .with_icon(&icons::X);
            vec![viewer, close]
        } else if self.players.loadable() {
            let play = MenuItem::plain(
                "menu-player-play",
                strings::embed_play(),
                action(PlayerAction::Play),
                false,
            )
            .with_icon(&icons::PLAY)
            .with_shortcut(strings::shortcut_space());
            vec![play, viewer]
        } else {
            Vec::new()
        }
    }

    /// The message menu's player items for message `id`'s first provider video, so the
    /// keyboard reaches them.
    pub(super) fn message_player_items(&self, key: TimelineKey, id: Uuid) -> Vec<MenuItem> {
        let Some(at) = self.first_provider_video(key, id) else {
            return Vec::new();
        };
        let Some(player) = self
            .preview_at(at)
            .as_ref()
            .and_then(|preview| player_of(at, preview).map(|(player, _)| player))
        else {
            return Vec::new();
        };
        self.player_items(at, player)
    }

    /// What the player menu's item does.
    pub(super) fn player_action(
        &mut self,
        at: MediaAt,
        key: PlayerKey,
        action: PlayerAction,
        tx: &mut Tx,
    ) {
        match action {
            PlayerAction::Play => self.play_player(at, tx),
            PlayerAction::OpenViewer => self.player_viewer_from(at, tx),
            PlayerAction::Close => self.close_player(at, key, tx),
        }
    }
}
