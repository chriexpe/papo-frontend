//! The GIF tab from the shell's side (`docs/design.md` §16.3, §16.5): opening it (the pill's GIF
//! button, Ctrl+G) on the account's favorites and recent GIFs, asking GIPHY for what a chip or
//! a search shows (only once the tab is open: nothing reaches GIPHY before), its keys, and
//! sending: a GIF leaves at once as its own message, its GIPHY link, which the server makes a
//! `gif` embed; an open reply applies to it and closes; the field's text stays.
//!
//! The tab's state is `gif_picker`'s; its tiles' places and pictures are `gif_tiles`'.

use papo_sdk::{AccountId, Gif, MessageTarget};
use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx};

use super::Shell;
use crate::composer::COMPOSER_PILL_KEY;
use crate::design::Look;
use crate::gif_picker::view::{GIF_SEARCH_KEY, GifLook, tile_key, tile_media};
use crate::gif_picker::{Ask, GifPicker, Page, SEARCH_AFTER_S, Shelf};
use crate::picker::{Frame, OpenPicker};

/// What the shell keeps for the GIF tab beyond the tab's own state.
pub(crate) struct GifSide {
    /// Where its tiles are, for which load and animate (`gif_tiles`).
    pub(super) tiles: super::gif_tiles::TilePlaces,
    /// The newest opening: only its library read opens the tab, and its asks are tagged from
    /// it, so an earlier opening's late answer is never taken.
    openings: u64,
    /// Whether this build can search GIPHY (its key is a build's); tests set it.
    pub(super) keyed: bool,
    /// Favorites on their way to the device (`gif_favorites`).
    pub(super) favorite_writes:
        std::collections::HashMap<(AccountId, String), super::gif_favorites::FavoriteWrite>,
}

impl GifSide {
    pub(super) fn new(keyed: bool) -> GifSide {
        GifSide {
            tiles: super::gif_tiles::TilePlaces::default(),
            openings: 0,
            keyed,
            favorite_writes: std::collections::HashMap::new(),
        }
    }

    /// An opening on its way (its library still being read) is dropped: the tab closed, the
    /// Emoji tab opened, or the conversation changed.
    pub(super) fn cancel_opening(&mut self) {
        self.openings = self.openings.wrapping_add(1);
    }

    /// The first ask tag of the opening now: openings tag their asks in ranges apart.
    fn first_tag(&self) -> u64 {
        self.openings.wrapping_shl(32)
    }
}

impl Shell {
    /// Whether the GIF tab is open: the pill's GIF button is lit.
    pub(crate) fn gif_open(&self) -> bool {
        self.gif_picker().is_some()
    }

    /// The pill's GIF button, or Ctrl+G: the GIF tab opens for the shown field (in the Emoji
    /// tab's place when that is open), or closes when it is the open one.
    pub(crate) fn gif_button_pressed(&mut self, tx: &mut Tx) {
        if self.gif_open() {
            self.close_emoji_picker(tx);
            return;
        }
        let Some((key, _)) = self
            .composer
            .shown()
            .filter(|_| !self.composer.is_loading())
        else {
            return;
        };
        let account = key.account;
        // The library is read first: the tab opens on Favorites when there are any.
        self.gif.cancel_opening();
        let opening = (self.gif.openings, key);
        let Some(sdk) = &self.sdk else {
            self.gif_library_read(opening, None, tx);
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let library = match client.session(account) {
                Ok(session) => session.expressions().snapshot().await.ok(),
                Err(_) => None,
            };
            updates.send_tx(move |shell, tx| {
                shell.gif_library_read(opening, library, tx);
                true
            });
        });
    }

    /// The account's library was read for opening `opening` (its number, and the field it
    /// was for): the tab opens, unless another opening, a close or another conversation came
    /// since. Without the library, it opens empty.
    fn gif_library_read(
        &mut self,
        (opening, key): (u64, crate::timeline::TimelineKey),
        library: Option<papo_sdk::ExpressionSnapshot>,
        tx: &mut Tx,
    ) {
        let same_field = self.composer.shown().is_some_and(|(shown, _)| shown == key);
        if opening != self.gif.openings || !same_field {
            return;
        }
        let account = key.account;
        let (favorites, recent) = library
            .map(|library| (library.favorite_gifs, library.recent_gifs))
            .unwrap_or_default();
        // It stands where the menu, a panel or the Emoji tab would: they close.
        self.drop_emoji_picker();
        self.worked_outside();
        self.nav.menu = None;
        self.stretched = None;
        let mut picker =
            GifPicker::new(self.gif.keyed, favorites, recent).tags_from(self.gif.first_tag());
        let first = picker.choose(picker.shelf.clone());
        self.picker = Some(OpenPicker::Gif(picker));
        self.gif.tiles.closed();
        if self.gif.keyed {
            self.ask_trending_searches(account);
            tx.focus(GIF_SEARCH_KEY, true);
        } else {
            // Without a search field, the keyboard starts on the chips.
            tx.focus("gif-chip-favorites", true);
        }
        if let Some(ask) = first {
            self.ask_giphy(account, ask);
        }
    }

    /// The search field's text, for its `on_edit`; a spare without the tab.
    pub(crate) fn gif_search_mut(&mut self) -> &mut TextEdit {
        if let Some(OpenPicker::Gif(picker)) = &mut self.picker {
            &mut picker.search
        } else {
            self.picker_spare = TextEdit::new("");
            &mut self.picker_spare
        }
    }

    /// The search changed: GIPHY is searched after a pause in typing.
    pub(crate) fn gif_searched(&mut self, tx: &mut Tx) {
        let clock_s = tx.clock();
        let Some(picker) = self.gif_picker_mut() else {
            return;
        };
        let back = picker.typed(clock_s);
        let waiting = picker.search_due_s.is_some();
        // Only a change of shelf changes the tiles; typing toward a search keeps them.
        if let Some(ask) = back {
            self.gif_tiles_changed();
            self.ask_giphy_shown(ask);
        }
        if waiting {
            tx.wake_after(std::time::Duration::from_secs_f64(SEARCH_AFTER_S));
        }
    }

    /// The search's ✕.
    pub(crate) fn gif_search_cleared(&mut self, tx: &mut Tx) {
        if let Some(picker) = self.gif_picker_mut() {
            picker.search = TextEdit::new("");
        }
        self.gif_searched(tx);
        tx.focus(GIF_SEARCH_KEY, false);
    }

    /// The window's clock moved on: a search whose pause is over goes to GIPHY. Returns whether
    /// the tab changed.
    pub(super) fn poll_gif_search(&mut self, clock_s: f64, tx: &mut Tx) -> bool {
        let Some(ask) = self.gif_picker_mut().and_then(|picker| picker.due(clock_s)) else {
            return false;
        };
        self.gif_tiles_changed();
        self.ask_giphy_shown(ask);
        tx.scroll_to(crate::gif_picker::view::GIF_SCROLL_KEY, 0.0, 0.0);
        true
    }

    /// A chip was pressed (a search chip fills the field).
    pub(crate) fn gif_chip_pressed(&mut self, chip: Shelf, tx: &mut Tx) {
        let Some(picker) = self.gif_picker_mut() else {
            return;
        };
        let ask = picker.choose(chip);
        self.gif_tiles_changed();
        if let Some(ask) = ask {
            self.ask_giphy_shown(ask);
        }
        tx.scroll_to(crate::gif_picker::view::GIF_SCROLL_KEY, 0.0, 0.0);
    }

    /// Try again, after GIPHY couldn't be reached.
    pub(crate) fn gif_try_again(&mut self, _tx: &mut Tx) {
        let ask = self.gif_picker_mut().and_then(GifPicker::try_again);
        if let Some(ask) = ask {
            self.ask_giphy_shown(ask);
        }
    }

    /// A key in the tab (§16.5), from its search or, without one, its chips: ↓ or Tab into the
    /// tiles, the arrows over them (← → one, ↑ ↓ along a column), Home / End to the first and
    /// last, Enter sends the chosen one (Shift+Enter keeps the tab open), Ctrl+D stars it,
    /// Escape clears the search first, then closes. Returns whether it was taken.
    pub(crate) fn gif_picker_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        // Ctrl+D, as a browser's bookmark key: the keyboard's tile (letters go to the search).
        let primary = (mods.ctrl || mods.cmd) && !mods.alt && !mods.shift && !mods.logo;
        if primary && key.is_command('d') {
            let Some(at) = self.gif_picker().and_then(|picker| picker.cursor) else {
                return false;
            };
            self.gif_tile_star_pressed(at, tx);
            return true;
        }
        if mods.ctrl || mods.cmd || mods.alt || mods.logo {
            return false;
        }
        let Some(picker) = self.gif_picker_mut() else {
            return false;
        };
        let in_tiles = picker.cursor.is_some();
        let plain = !mods.shift;
        match key.key {
            Key::Escape if !mods.shift => {
                if picker.search.text.is_empty() {
                    self.close_emoji_picker(tx);
                } else {
                    self.gif_search_cleared(tx);
                }
                return true;
            }
            Key::Enter => {
                if let Some(at) = picker.cursor {
                    self.gif_pressed(at, mods.shift, tx);
                    return true;
                }
                return false;
            }
            Key::Tab if plain && !in_tiles => {
                picker.step(1);
            }
            Key::Tab => {
                picker.cursor = None;
                return false;
            }
            Key::Down if plain => {
                picker.step_vertical(true);
            }
            Key::Up if plain && in_tiles => {
                picker.step_vertical(false);
            }
            Key::Left if plain && in_tiles => {
                picker.step(-1);
            }
            Key::Right if plain && in_tiles => {
                picker.step(1);
            }
            Key::Home if plain && in_tiles => {
                picker.step_to_end(false);
            }
            Key::End if plain && in_tiles => {
                picker.step_to_end(true);
            }
            _ => return false,
        }
        self.gif_cursor_moved(tx);
        true
    }

    /// The keyboard's tile changed: it comes into view, and it plays as the pointer's would
    /// when GIFs don't animate on their own (§16.3: "plays on hover or focus").
    fn gif_cursor_moved(&mut self, tx: &mut Tx) {
        let Some(account) = self.composer.shown().map(|(key, _)| key.account) else {
            return;
        };
        let chosen = self.gif_picker().and_then(|picker| {
            let at = picker.cursor?;
            Some((at, picker.gifs().get(at)?.clone()))
        });
        match chosen {
            Some((at, gif)) => {
                tx.scroll_into_view(tile_key(at, &gif));
                self.gif_tile_hovered(tile_media(account, &gif), true);
            }
            None => self.gif_tile_unhovered(),
        }
    }

    /// The star on tile `at`, or Ctrl+D on the keyboard's tile: its GIF becomes a favorite or
    /// stops being one. The tiles stay where they are until the tab opens again (§16.4).
    pub(crate) fn gif_tile_star_pressed(&mut self, at: usize, tx: &mut Tx) {
        let (Some(gif), Some((key, _))) = (
            self.gif_picker()
                .and_then(|picker| picker.gifs().get(at).cloned()),
            self.composer.shown(),
        ) else {
            return;
        };
        self.toggle_gif_favorite(key.account, gif, tx);
    }

    /// Tile `at` was pressed (or Enter on it): the GIF is sent at once, as its own message, to
    /// the shown conversation; an open reply applies to it and closes. Shift (`keep`) keeps the
    /// tab open.
    pub(crate) fn gif_pressed(&mut self, at: usize, keep: bool, tx: &mut Tx) {
        let Some(gif) = self
            .gif_picker()
            .and_then(|picker| picker.gifs().get(at).cloned())
        else {
            return;
        };
        let Some((key, draft)) = self.composer.shown() else {
            return;
        };
        let reply_to = draft.mode.reply().map(|reply| reply.id);
        // A GIF that can't be sent leaves the reply and the tab as they were, and says so.
        if !self.send_gif(key, &gif, reply_to) {
            self.refuse(crate::notice::Refusal::Gif, tx);
            return;
        }
        if reply_to.is_some()
            && let Some(effects) = self.composer.cancel_reply()
        {
            self.run_composer(effects, Some(tx));
        }
        if keep {
            tx.focus(GIF_SEARCH_KEY, false);
        } else {
            self.close_emoji_picker(tx);
        }
    }

    /// Queues `gif` in conversation `key` as its own message, a reply to `reply_to` when one
    /// was being written. Returns whether it went to the queue; one the queue refuses later is
    /// said, as a refused send is.
    fn send_gif(
        &mut self,
        key: crate::timeline::TimelineKey,
        gif: &Gif,
        reply_to: Option<papo_sdk::Uuid>,
    ) -> bool {
        let (Some(sdk), Ok(mut message)) = (&self.sdk, gif.message()) else {
            return false;
        };
        message.reply_to = reply_to.map(MessageTarget::Remote);
        let local = (self.local_ids)();
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let queued = match client.session(key.account) {
                Ok(session) => {
                    super::draft_writes::queue_message(&session, key.channel, local, message)
                        .await
                        .map_err(|error| error.to_string())
                }
                Err(error) => Err(error.to_string()),
            };
            if let Err(error) = queued {
                log::warn!("a GIF was not queued: {error}");
                updates.send_tx(|shell, tx| {
                    shell.refuse(crate::notice::Refusal::Gif, tx);
                    true
                });
            }
        });
        true
    }

    /// Asks GIPHY for `ask`, as the shown conversation's account.
    fn ask_giphy_shown(&mut self, ask: Ask) {
        match self.composer.shown().map(|(key, _)| key.account) {
            Some(account) => self.ask_giphy(account, ask),
            None => self.ask_dropped(&ask),
        }
    }

    /// `ask` can't go out: the tab stops waiting for it.
    fn ask_dropped(&mut self, ask: &Ask) {
        if let Some(picker) = self.gif_picker_mut() {
            picker.landed(ask.tag, None);
        }
    }

    /// Asks GIPHY for `ask` (Trending's page, or a search's) as `account`; the answer lands in
    /// the tab, unless a newer ask went out since.
    pub(super) fn ask_giphy(&mut self, account: AccountId, ask: Ask) {
        let Some(sdk) = &self.sdk else {
            self.ask_dropped(&ask);
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let page = match client.session(account) {
                Ok(session) => {
                    let expressions = session.expressions();
                    let answered = match ask.query {
                        Some(query) => expressions.search(query, ask.offset).await,
                        None => expressions.trending(ask.offset).await,
                    };
                    answered.ok().map(|page| Page {
                        gifs: page.value.gifs,
                        next_offset: page.value.next_offset,
                    })
                }
                Err(_) => None,
            };
            updates.send(move |shell| {
                shell
                    .gif_picker_mut()
                    .is_some_and(|picker| picker.landed(ask.tag, page))
            });
        });
    }

    /// Asks GIPHY for its trending searches, the chips after Trending.
    fn ask_trending_searches(&mut self, account: AccountId) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let searches = match client.session(account) {
                Ok(session) => session.expressions().trending_searches().await.ok(),
                Err(_) => None,
            };
            let Some(searches) = searches else {
                return;
            };
            updates.send(move |shell| {
                let Some(picker) = shell.gif_picker_mut() else {
                    return false;
                };
                picker.searches_landed(searches.value);
                true
            });
        });
    }
}

/// The GIF tab drawn (§16.3): grown from the text pill, as wide as it at most.
pub(super) fn view(shell: &Shell, picker: &GifPicker, look: &Look, cx: &Cx<Shell>) -> El {
    // The tab is the shown field's: it closes when the conversation goes.
    let Some(account) = shell.composer.shown().map(|(key, _)| key.account) else {
        return rune::el();
    };
    let conversation_height_pt = shell.conversation_rect[3];
    let media = shell.media.view();
    let given = GifLook {
        frame: Frame {
            look,
            anchor: COMPOSER_PILL_KEY,
            anchor_width_pt: cx.layout_size(COMPOSER_PILL_KEY).map(|[width, _]| width),
            conversation_height_pt: (conversation_height_pt > 0.0)
                .then_some(conversation_height_pt),
        },
        media: &media,
        account,
        favorites: shell.favorite_gifs(account),
    };
    crate::gif_picker::view::view(picker, &given)
}
