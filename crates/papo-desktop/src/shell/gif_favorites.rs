//! GIF favorites from the shell's side (`docs/design.md` §16.4): whether a GIF is one (the
//! account's library, as its watch last posted it), and starring or unstarring one, from the
//! star on a conversation GIF or a picker tile, S on a focused GIF message, Ctrl+D in the
//! picker, or the message and media menus. Adding is said once, politely. The favorite itself
//! is the SDK's (`ExpressionService::favorite_gif`, kept per account on the device).

use std::collections::HashSet;
use std::sync::Arc;

use papo_sdk::{AccountId, Gif, Message, Uuid};
use rune::{LiveRegion, Tx};

use super::Shell;
use crate::icons;
use crate::menu::{MenuAction, MenuItem};
use crate::notice::Refusal;
use crate::strings;
use crate::timeline::TimelineKey;

impl Shell {
    /// Message `id`'s GIF, in the shown conversation `key`, becomes a favorite or stops being
    /// one (S, the menus). Returns whether the message carries a GIF that takes a star.
    pub(crate) fn star_message_gif(&mut self, key: TimelineKey, id: Uuid, tx: &mut Tx) -> bool {
        let Some(gif) = self
            .tabs
            .shown()
            .filter(|open| open.key == key)
            .and_then(|open| open.message(id))
            .and_then(|(message, _)| papo_sdk_ui::favorite_gif(message))
        else {
            return false;
        };
        self.toggle_gif_favorite(key.account, gif, tx);
        true
    }

    /// The menus' item for `message`'s GIF (message `id` in `key`): "Add to favorites" or
    /// "Remove from favorites", with S; none for a message whose GIF takes no star.
    pub(super) fn favorite_item(
        &self,
        key: TimelineKey,
        id: Uuid,
        message: &Message,
    ) -> Option<MenuItem> {
        let gif = papo_sdk_ui::favorite_gif(message)?;
        let (name, label, shape) = if self.is_favorite_gif(key.account, &gif) {
            (
                "menu-fav-remove",
                strings::fav_remove(),
                &icons::STAR_FILLED,
            )
        } else {
            ("menu-fav-add", strings::fav_add(), &icons::STAR)
        };
        Some(
            MenuItem::plain(name, label, MenuAction::FavoriteGif { key, id }, false)
                .with_icon(shape)
                .with_shortcut(strings::shortcut_favorite()),
        )
    }

    /// `account`'s favorite GIFs, by GIPHY's ID.
    pub(crate) fn favorite_gifs(&self, account: AccountId) -> Arc<HashSet<String>> {
        self.activity
            .get(&account)
            .map(|activity| Arc::clone(&activity.favorite_gifs))
            .unwrap_or_default()
    }

    /// Whether `gif` is one of `account`'s favorites.
    pub(crate) fn is_favorite_gif(&self, account: AccountId, gif: &Gif) -> bool {
        self.favorite_gifs(account).contains(&gif.id)
    }

    /// `gif` becomes one of `account`'s favorites, or stops being one. The star shows it at
    /// once; the library's watch confirms it, and a refusal is said and put back.
    pub(crate) fn toggle_gif_favorite(&mut self, account: AccountId, gif: Gif, tx: &mut Tx) {
        let favorite = !self.is_favorite_gif(account, &gif);
        let activity = self.activity.entry(account).or_default();
        let favorites = Arc::make_mut(&mut activity.favorite_gifs);
        if favorite {
            favorites.insert(gif.id.clone());
            tx.announce(strings::fav_added(), LiveRegion::Polite);
        } else {
            favorites.remove(&gif.id);
        }
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        let id = gif.id.clone();
        sdk.run(move |client| async move {
            let kept = match client.session(account) {
                Ok(session) => session
                    .expressions()
                    .favorite_gif(gif, favorite)
                    .await
                    .is_ok(),
                Err(_) => false,
            };
            if kept {
                return;
            }
            updates.send_tx(move |shell, tx| {
                // Put back what the star showed, and say so.
                let activity = shell.activity.entry(account).or_default();
                let favorites = Arc::make_mut(&mut activity.favorite_gifs);
                if favorite {
                    favorites.remove(&id);
                } else {
                    favorites.insert(id);
                }
                shell.refuse(Refusal::Favorite, tx);
                true
            });
        });
    }
}
