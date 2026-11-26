//! GIF favorites from the shell's side (`docs/design.md` §16.4): whether a GIF is one (the
//! account's library, as its watch last posted it), and starring or unstarring one, from the
//! star on a conversation GIF or a picker tile, S on a focused GIF message, Ctrl+D in the
//! picker, or the message and media menus. Adding is said once, politely. Writes for one GIF go
//! one at a time, in order, and the library's posts don't undo one on its way. The favorite
//! itself is the SDK's (`ExpressionService::favorite_gif`, kept per account on the device).

use std::collections::HashSet;
use std::sync::Arc;

use papo_sdk::{AccountId, Gif, Message, Uuid};

use crate::message_media::received::MediaAt;
use rune::{LiveRegion, Tx};

use super::Shell;
use crate::icons;
use crate::menu::{MenuAction, MenuItem};
use crate::notice::Refusal;
use crate::strings;
use crate::timeline::TimelineKey;

/// A GIF's favorite on its way to the device, by account and GIPHY's ID.
pub(crate) struct FavoriteWrite {
    /// What the star shows, and the last write will have kept.
    wanted: bool,
    gif: Gif,
    /// A write is on its way: a toggle meanwhile waits for it.
    writing: bool,
}

impl Shell {
    /// Message `id`'s GIF, in the shown conversation `key`, becomes a favorite or stops being
    /// one (S, the menus): the picture of embed `embed`, else its first GIF. Returns whether
    /// the message carries a GIF that takes a star.
    pub(crate) fn star_message_gif(
        &mut self,
        key: TimelineKey,
        id: Uuid,
        embed: Option<Uuid>,
        tx: &mut Tx,
    ) -> bool {
        let Some(gif) = self
            .tabs
            .shown()
            .filter(|open| open.key == key)
            .and_then(|open| open.message(id))
            .and_then(|(message, _)| match embed {
                Some(embed) => crate::gif_star::preview_gif(message, embed),
                None => papo_sdk_ui::favorite_gif(message),
            })
        else {
            return false;
        };
        self.toggle_gif_favorite(key.account, gif, tx);
        true
    }

    /// The GIF the picture at `at` shows, when it takes a star: a GIF link's preview.
    pub(crate) fn media_gif(&self, at: MediaAt) -> Option<Gif> {
        self.media_gif_of(at).map(|(_, gif)| gif)
    }

    /// The picture at `at`'s embed and GIF, when it takes a star.
    pub(super) fn media_gif_of(&self, at: MediaAt) -> Option<(Uuid, Gif)> {
        let Some(papo_sdk_ui::LinkPreview::Media(media)) = self.preview_at(at) else {
            return None;
        };
        let message = self.message_at(at)?;
        let gif = crate::gif_star::preview_gif(&message, media.embed)?;
        Some((media.embed, gif))
    }

    /// The message menu's item for `message`'s GIF (message `id` in `key`): "Add to favorites"
    /// or "Remove from favorites", with S; none for a message whose GIF takes no star.
    pub(super) fn favorite_item(
        &self,
        key: TimelineKey,
        id: Uuid,
        message: &Message,
    ) -> Option<MenuItem> {
        let gif = papo_sdk_ui::favorite_gif(message)?;
        Some(self.favorite_item_for(key, id, None, &gif))
    }

    /// The menus' item for `gif` in message `id` of `key`, the picture of embed `embed` when a
    /// media menu offers it (else the message's first GIF).
    pub(super) fn favorite_item_for(
        &self,
        key: TimelineKey,
        id: Uuid,
        embed: Option<Uuid>,
        gif: &Gif,
    ) -> MenuItem {
        let (name, label, shape) = if self.is_favorite_gif(key.account, gif) {
            (
                "menu-fav-remove",
                strings::fav_remove(),
                &icons::STAR_FILLED,
            )
        } else {
            ("menu-fav-add", strings::fav_add(), &icons::STAR)
        };
        MenuItem::plain(
            name,
            label,
            MenuAction::FavoriteGif { key, id, embed },
            false,
        )
        .with_icon(shape)
        .with_shortcut(strings::shortcut_favorite())
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
    /// once. Writes for one GIF go one at a time, in order: a toggle while one is on its way
    /// only changes what the next one writes. Adding is said once, politely.
    pub(crate) fn toggle_gif_favorite(&mut self, account: AccountId, gif: Gif, tx: &mut Tx) {
        let favorite = !self.is_favorite_gif(account, &gif);
        self.set_shown_favorite(account, &gif.id, favorite);
        if favorite {
            tx.announce(strings::fav_added(), LiveRegion::Polite);
        }
        let write = self
            .gif
            .favorite_writes
            .entry((account, gif.id.clone()))
            .or_insert(FavoriteWrite {
                wanted: favorite,
                gif: gif.clone(),
                writing: false,
            });
        write.wanted = favorite;
        write.gif = gif;
        let start = (!write.writing).then(|| {
            write.writing = true;
            write.gif.clone()
        });
        if let Some(gif) = start {
            self.write_favorite(account, gif, favorite);
        }
    }

    /// The star shows `id` as a favorite of `account`'s (`favorite`), or not.
    fn set_shown_favorite(&mut self, account: AccountId, id: &str, favorite: bool) {
        let activity = self.activity.entry(account).or_default();
        let favorites = Arc::make_mut(&mut activity.favorite_gifs);
        if favorite {
            favorites.insert(id.to_owned());
        } else {
            favorites.remove(id);
        }
    }

    /// Writes `gif`'s favorite (`favorite`) to the device; it must land, even as the app quits.
    fn write_favorite(&mut self, account: AccountId, gif: Gif, favorite: bool) {
        let Some(sdk) = &self.sdk else {
            self.gif.favorite_writes.remove(&(account, gif.id));
            return;
        };
        let updates = sdk.updates();
        sdk.finish(move |client| async move {
            let id = gif.id.clone();
            let kept = match client.session(account) {
                Ok(session) => session
                    .expressions()
                    .favorite_gif(gif, favorite)
                    .await
                    .is_ok(),
                Err(_) => false,
            };
            updates.send_tx(move |shell, tx| {
                shell.favorite_written(account, &id, favorite, kept, tx);
                true
            });
        });
    }

    /// A write of `id`'s favorite (`favorite`) landed (`kept`), or didn't. If a later toggle
    /// wants otherwise, the next write goes out; a refusal is said and the device's library is
    /// read again, so the stars show what it keeps.
    fn favorite_written(
        &mut self,
        account: AccountId,
        id: &str,
        favorite: bool,
        kept: bool,
        tx: &mut Tx,
    ) {
        let key = (account, id.to_owned());
        let Some(write) = self.gif.favorite_writes.get(&key) else {
            return;
        };
        if !kept {
            self.gif.favorite_writes.remove(&key);
            self.refuse(Refusal::Favorite, tx);
            self.reread_favorites(account);
            return;
        }
        if write.wanted == favorite {
            self.gif.favorite_writes.remove(&key);
            return;
        }
        let (gif, wanted) = (write.gif.clone(), write.wanted);
        self.write_favorite(account, gif, wanted);
    }

    /// Reads `account`'s favorites from the device again, over what the stars showed.
    fn reread_favorites(&mut self, account: AccountId) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let Some(library) = (match client.session(account) {
                Ok(session) => session.expressions().snapshot().await.ok(),
                Err(_) => None,
            }) else {
                return;
            };
            let favorites: HashSet<String> = library
                .favorite_gifs
                .into_iter()
                .map(|gif| gif.id)
                .collect();
            updates.send(move |shell| {
                shell.favorites_posted(account, favorites);
                true
            });
        });
    }

    /// The library's watch (or a read) says `account`'s favorites are `favorites`: the stars
    /// follow, but for GIFs whose writes are still on their way, which show what was asked.
    pub(super) fn favorites_posted(&mut self, account: AccountId, mut favorites: HashSet<String>) {
        for ((of, id), write) in &self.gif.favorite_writes {
            if *of != account {
                continue;
            }
            if write.wanted {
                favorites.insert(id.clone());
            } else {
                favorites.remove(id);
            }
        }
        self.activity.entry(account).or_default().favorite_gifs = Arc::new(favorites);
    }
}
