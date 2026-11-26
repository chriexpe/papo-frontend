//! The search panel's searches (`docs/design.md` §10, Search): the field's
//! text is searched once it has rested for [`RUN_AFTER_S`] (Christian,
//! 2026-11-23: as you type), or at once on Enter. While the next answer
//! loads, the results shown stay (the panel never empties between two
//! searches). Pages and opening a result are `search_open`'s.
//!
//! Every search takes the next number of one counter for the whole shell,
//! so an answer for a search that was replaced, stopped or dropped (with
//! its account, server or DM) never lands in another; its request is
//! cancelled. A replaced search is never shown as a failure. The query's
//! text is never logged.

use std::time::Duration;

use papo_sdk::{SearchError, SearchPage, SearchProblem, SearchRequestHandle};
use papo_sdk_ui::{SearchResults, localize_search};
use rune::Tx;

use super::Shell;
use super::search::{Found, Paging};

/// How long the text rests before it is searched, in seconds.
pub(crate) const RUN_AFTER_S: f64 = 0.3;

impl Shell {
    /// The field's text changed: it is searched once it rests.
    pub(crate) fn search_edited(&mut self, tx: &mut Tx) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        search.highlight = 0;
        search.run_due_s = Some(tx.clock() + RUN_AFTER_S);
        // Each edit asks its own wake; an earlier one finds the text not
        // rested yet and waits for this one.
        tx.wake_after(Duration::from_secs_f64(RUN_AFTER_S));
    }

    /// The window's poll: a text that has rested is searched. Returns
    /// whether the view changed.
    pub(super) fn poll_search(&mut self, clock_s: f64) -> bool {
        let due = self
            .search
            .as_ref()
            .and_then(|search| search.run_due_s)
            .is_some_and(|due_s| due_s <= clock_s);
        if due {
            self.search_now();
        }
        due
    }

    /// Searches the field's text now, unless it is what the results
    /// already answer; text that asks for no search yet holds it.
    pub(crate) fn search_now(&mut self) {
        let draft = self.search_draft();
        let offsets = std::sync::Arc::clone(&self.local_time.offsets);
        let Some(search) = self.search.as_mut() else {
            return;
        };
        search.run_due_s = None;
        let query = match draft.as_ref().map(|draft| draft.query()) {
            Some(Ok(query)) => query.clone(),
            Some(Err(SearchProblem::Empty)) | None => {
                search.stop();
                search.found = Found::Nothing;
                return;
            }
            Some(Err(_)) => {
                search.stop();
                search.found = Found::Held;
                return;
            }
        };
        // In flight or answered for this very text: nothing to ask.
        if search.revision != 0 && search.asked.as_ref() == Some(&query) {
            return;
        }
        // Days of the reader's calendar become the instants they span,
        // read in the zone the conversation shows times in.
        let Some((request, window)) = localize_search(query.clone(), &*offsets) else {
            search.stop();
            search.found = Found::Held;
            return;
        };
        search.stop();
        self.search_runs += 1;
        search.revision = self.search_runs;
        search.asked = Some(query);
        search.window = window;
        search.opened = None;
        // The results shown stay while the next ones load.
        if !matches!(search.found, Found::Ready(_)) {
            search.found = Found::Loading;
        }
        let (account, revision) = (search.account, search.revision);
        let window = search.window;
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let request = client
                .session(account)
                .map_err(SearchError::from)
                .and_then(|session| session.search(request));
            let request = match request {
                Ok(request) => request,
                Err(error) => {
                    updates.send(move |shell| shell.search_answered(revision, Err(error)));
                    return;
                }
            };
            // Kept at once, so stopping the search cancels its request.
            let kept = request.clone();
            updates.send(move |shell| shell.search_requested(revision, kept));
            // A date search starts at its window's near end.
            let page = request.page(window.first_cursor()).await;
            updates.send(move |shell| shell.search_answered(revision, page));
        });
    }

    /// The search, while `revision` is its current search.
    pub(super) fn current_search(&mut self, revision: u64) -> Option<&mut super::search::Search> {
        self.search
            .as_mut()
            .filter(|search| revision != 0 && search.revision == revision)
    }

    /// Search `revision`'s request exists: kept for stopping and paging,
    /// or cancelled when the search has moved on.
    fn search_requested(&mut self, revision: u64, request: SearchRequestHandle) -> bool {
        match self.current_search(revision) {
            Some(search) => search.request = Some(request),
            None => request.cancel(),
        }
        false
    }

    /// The first page of search `revision` arrived. Returns whether the
    /// view changed.
    pub(super) fn search_answered(
        &mut self,
        revision: u64,
        answer: Result<SearchPage, SearchError>,
    ) -> bool {
        let Some(account) = self.current_search(revision).map(|search| search.account) else {
            return false;
        };
        let emojis = self.custom_emojis(account);
        let Some(search) = self.current_search(revision) else {
            return false;
        };
        search.paging = Paging::Idle;
        match answer {
            Ok(page) => {
                search.generation = Some(page.generation);
                let mut results = SearchResults::first_within(&page, search.window);
                // Its `:name:` draw as the account's emoji, and so do later pages'.
                results.set_custom_emojis(&emojis);
                search.found = Found::Ready(results);
            }
            // The search is still current, so the SDK cancelled it (its
            // connection went): said as a failure, never left waiting.
            Err(error) => {
                log::warn!("a search failed: {error}");
                search.found = Found::Failed;
            }
        }
        search.answered = revision;
        self.search_more_if_trimmed();
        true
    }

    /// "Try again" after a failure: the same text is searched again.
    pub(crate) fn search_retry(&mut self) {
        if let Some(search) = self.search.as_mut() {
            search.stop();
            search.asked = None;
        }
        self.search_now();
    }
}
