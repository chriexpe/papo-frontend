//! The search panel from the shell's side (`docs/design.md` §10, Search
//! and `PanelPeek`): 🔍 or Ctrl+F opens it out of the actions pill with
//! the keyboard in its field, the search runs as you type
//! (`search_run`), and a result opens in the conversation
//! (`message_open`) while the panel marks it.
//!
//! It shares the pins panel's rule (`pins`): input that starts outside it
//! shrinks it to the opened result, or folds it back into the actions
//! pill when nothing was opened, keeping the query; coming back stretches
//! it. Only one header panel is open at a time. A search belongs to the
//! account and the server or DM it was opened on, and goes when another
//! account, server or DM shows; one scoped to "This channel" goes when you
//! move to another channel yourself (Christian, 2026-11-23). Escape folds
//! it, giving the keyboard back.

use papo_sdk::{AccountId, SearchQuery, SearchRequestHandle, SearchScope, Uuid};
use papo_sdk_ui::SearchResults;
use rune::{TextEdit, Tx};

use super::Shell;

/// The element key of 🔍, which the panel grows from.
pub(crate) const SEARCH_BUTTON_KEY: &str = "search-button";
/// The element key of the open panel.
pub(crate) const SEARCH_PANEL_KEY: &str = "search-panel";
/// The element key of the panel's field.
pub(crate) const SEARCH_FIELD_KEY: &str = "search-field";

/// The element key of result `id`'s row.
pub(crate) fn hit_row_key(id: Uuid) -> String {
    format!("search-hit-{id}")
}

/// Where a search was opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Place {
    /// A server's channel: the scope choice says the server or this channel.
    Server { channel: Uuid },
    /// A DM: the search covers it alone.
    Direct { channel: Uuid },
}

/// The scope choice on a server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Server,
    Channel,
}

/// How the panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Showing {
    /// Folded back into the actions pill, its query kept.
    Folded,
    Open,
    /// Shrunk to its field and the opened result.
    Peek,
}

/// What the results area holds.
pub(crate) enum Found {
    /// Nothing typed, or nothing asked yet.
    Nothing,
    /// The text asks for no search yet (an ambiguous or unknown name, an
    /// open quote): nothing is searched until it does.
    Held,
    Loading,
    Ready(SearchResults),
    Failed,
}

/// The next page of results.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Paging {
    Idle,
    Loading,
    /// The last page failed; its Try again row asks again.
    Failed,
}

/// A search, open or folded.
pub(crate) struct Search {
    pub(crate) account: AccountId,
    pub(crate) place: Place,
    pub(crate) scope: Scope,
    pub(crate) edit: TextEdit,
    pub(crate) showing: Showing,
    pub(crate) found: Found,
    pub(crate) paging: Paging,
    /// The query the results answer, or are being asked for.
    pub(super) asked: Option<SearchQuery>,
    /// The backend request the results came from, for their next pages.
    pub(super) request: Option<SearchRequestHandle>,
    /// The current search's number from the shell's counter, 0 when none
    /// is asked; only its answers count.
    pub(super) revision: u64,
    /// The number of the search the results shown answer.
    pub(super) answered: u64,
    /// The instants the current search keeps (its date filters' exact
    /// days in the reader's zone).
    pub(super) window: papo_sdk_ui::SearchWindow,
    /// The connection generation the results came from: a reconnect
    /// since makes them stale.
    pub(super) generation: Option<papo_sdk::Generation>,
    /// When the text last changed is due to be searched, on the window's
    /// clock.
    pub(super) run_due_s: Option<f64>,
    /// The highlighted row of the list under the field.
    pub(crate) highlight: usize,
    /// The result opened from it, which it marks.
    pub(crate) opened: Option<Uuid>,
    /// What had the keyboard before it opened, which gets it back.
    pub(super) return_focus: Option<String>,
    /// The results on screen hydrated into whole messages, for their media and cards.
    pub(crate) hydrated: super::search_hydration::Hydrated,
}

impl Search {
    fn new(account: AccountId, place: Place) -> Search {
        Search {
            account,
            place,
            scope: Scope::Server,
            edit: TextEdit::new(""),
            showing: Showing::Open,
            found: Found::Nothing,
            paging: Paging::Idle,
            asked: None,
            request: None,
            revision: 0,
            answered: 0,
            window: papo_sdk_ui::SearchWindow::default(),
            generation: None,
            run_due_s: None,
            highlight: 0,
            opened: None,
            return_focus: None,
            hydrated: super::search_hydration::Hydrated::default(),
        }
    }

    /// Where the SDK looks, from the place and the scope choice.
    pub(crate) fn sdk_scope(&self) -> SearchScope {
        match (self.place, self.scope) {
            (Place::Direct { channel }, _) => SearchScope::Direct {
                channel,
                account_user: self.account.user(),
            },
            (Place::Server { .. }, Scope::Server) => SearchScope::Server,
            (Place::Server { channel }, Scope::Channel) => SearchScope::Channel(channel),
        }
    }

    /// Whether a search is asked whose answer has not come: the results
    /// shown, if any, are the previous search's.
    pub(crate) fn loading(&self) -> bool {
        self.revision != 0 && self.answered != self.revision
    }

    /// Stops the request in flight and forgets the results' source; no
    /// answer of the stopped search counts any more.
    pub(super) fn stop(&mut self) {
        self.revision = 0;
        if let Some(request) = self.request.take() {
            request.cancel();
        }
        self.asked = None;
        self.paging = Paging::Idle;
        self.run_due_s = None;
    }
}

impl Shell {
    /// 🔍 or Ctrl+F (`keyboard`): the panel opens with the keyboard in its
    /// field (a folded one with its query), a shrunk one stretches back,
    /// and 🔍 on an open one folds it.
    pub(crate) fn search_pressed(&mut self, keyboard: bool, tx: &mut Tx) {
        // A press outside the picker closes it (§16.1); this one skips `worked_outside`.
        self.drop_emoji_picker();
        let Some(key) = self.tabs.shown().map(|open| open.key) else {
            return;
        };
        if let Some(search) = self
            .search
            .as_mut()
            .filter(|search| search.account == key.account)
        {
            match search.showing {
                Showing::Open if !keyboard => {
                    self.close_search(tx);
                    return;
                }
                Showing::Open => {}
                Showing::Peek => {
                    search.showing = Showing::Open;
                    if let Some(id) = search.opened {
                        tx.scroll_into_view(hit_row_key(id));
                    }
                }
                Showing::Folded => {
                    search.showing = Showing::Open;
                    search.return_focus = tx.focused().map(str::to_owned);
                }
            }
        } else {
            let place = if self.nav.direct_shown() {
                Place::Direct {
                    channel: key.channel,
                }
            } else {
                Place::Server {
                    channel: key.channel,
                }
            };
            let mut search = Search::new(key.account, place);
            search.return_focus = tx.focused().map(str::to_owned);
            if let Some(mut old) = self.search.replace(search) {
                old.stop();
            }
        }
        // One header panel at a time.
        if self.pins_panel.take().is_some() {
            self.cancel_opening();
        }
        self.nav.menu = None;
        tx.focus(SEARCH_FIELD_KEY, true);
    }

    /// Escape, or 🔍 on the open panel: it folds, keeping its query, and
    /// what had the keyboard before gets it back.
    pub(crate) fn close_search(&mut self, tx: &mut Tx) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        if search.showing == Showing::Folded {
            return;
        }
        search.showing = Showing::Folded;
        let focus = search.return_focus.take();
        self.cancel_opening();
        match focus {
            Some(key) => tx.focus(key, true),
            None => tx.focus(SEARCH_BUTTON_KEY, true),
        }
    }

    /// Pins opened, or Escape: an open search folds, and a result still
    /// opening from it no longer moves the conversation.
    pub(super) fn fold_search(&mut self) {
        if let Some(search) = self.search.as_mut()
            && search.showing != Showing::Folded
        {
            search.showing = Showing::Folded;
            self.cancel_opening();
        }
    }

    /// The field's ✕: the text, its results and the opened result go.
    pub(crate) fn clear_search(&mut self, tx: &mut Tx) {
        if let Some(search) = self.search.as_mut() {
            search.stop();
            search.edit = TextEdit::new("");
            search.found = Found::Nothing;
            search.opened = None;
            search.highlight = 0;
        }
        tx.focus(SEARCH_FIELD_KEY, true);
    }

    /// The field Rune edits; without a search, a spare nothing reads (as
    /// the composer's `field_mut`).
    pub(crate) fn search_field_mut(&mut self) -> &mut TextEdit {
        let Shell {
            search,
            search_spare,
            ..
        } = self;
        search.as_mut().map_or_else(
            || {
                *search_spare = TextEdit::new("");
                search_spare
            },
            |search| &mut search.edit,
        )
    }

    /// The panel, when it shows over the shown conversation.
    pub(crate) fn shown_search(&self) -> Option<&Search> {
        let shown = self.tabs.shown()?.key;
        self.search
            .as_ref()
            .filter(|search| search.showing != Showing::Folded && search.account == shown.account)
    }

    /// Input outside the panel: it shrinks to the opened result, or folds
    /// when nothing was opened. Returns whether it changed.
    pub(super) fn search_worked_outside(&mut self) -> bool {
        let Some(search) = self.search.as_mut() else {
            return false;
        };
        let next = match (search.showing, search.opened) {
            (Showing::Open, Some(_)) => Showing::Peek,
            (Showing::Open, None) => Showing::Folded,
            (showing, _) => showing,
        };
        std::mem::replace(&mut search.showing, next) != next
    }

    /// The pointer left the panel: once a result was opened, it shrinks to
    /// it (the conversation is where you work now).
    pub(crate) fn search_left(&mut self) {
        if let Some(search) = self
            .search
            .as_mut()
            .filter(|search| search.opened.is_some() && search.showing == Showing::Open)
        {
            search.showing = Showing::Peek;
        }
    }

    /// The pointer came onto the panel: a shrunk one stretches back, the
    /// opened result in view. Returns whether it changed.
    pub(crate) fn search_hovered(&mut self, tx: &mut Tx) -> bool {
        let Some(search) = self
            .search
            .as_mut()
            .filter(|search| search.showing == Showing::Peek)
        else {
            return false;
        };
        search.showing = Showing::Open;
        if let Some(id) = search.opened {
            tx.scroll_into_view(hit_row_key(id));
        }
        true
    }

    /// Another conversation shows: a search that does not belong to it
    /// goes (another account, server or DM; a This-channel search on
    /// another channel, unless that channel shows because a result of it
    /// was opened).
    pub(super) fn search_conversation_left(&mut self) {
        let Some(search) = self.search.as_ref() else {
            return;
        };
        let shown = self.tabs.shown().map(|open| open.key);
        let opened_channel = search.opened.and_then(|id| match &search.found {
            Found::Ready(results) => results
                .items()
                .iter()
                .find(|item| item.id() == id)
                .map(|item| item.channel),
            _ => None,
        });
        let belongs = shown.is_some_and(|shown| {
            if shown.account != search.account {
                return false;
            }
            match search.place {
                Place::Direct { channel } => shown.channel == channel,
                Place::Server { .. } if self.nav.direct_shown() => false,
                Place::Server { channel } => {
                    search.scope == Scope::Server
                        || shown.channel == channel
                        || opened_channel == Some(shown.channel)
                }
            }
        });
        if !belongs {
            if let Some(mut search) = self.search.take() {
                search.stop();
            }
            return;
        }
        // A server search kept on another channel: "This channel" is the
        // one showing now.
        if let (Some(search), Some(shown)) = (self.search.as_mut(), shown)
            && search.scope == Scope::Server
            && let Place::Server { channel } = &mut search.place
        {
            *channel = shown.channel;
        }
    }
}
