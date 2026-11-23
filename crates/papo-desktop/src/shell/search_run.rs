//! The search panel's requests (`docs/design.md` §10, Search): the field's
//! text is searched once it has rested for [`RUN_AFTER_S`] (Christian,
//! 2026-11-23: as you type), or at once on Enter; the next page is asked
//! as the end of the results comes near; an opened result is checked
//! against the account's connection first, and results a reconnect made
//! stale are searched again.
//!
//! A search the text replaced is dropped and cancelled, never shown as a
//! failure. The query's text is never logged.

use std::ops::Range;
use std::time::Duration;

use papo_sdk::{SearchError, SearchPage, SearchProblem, SearchRequestHandle, Uuid};
use papo_sdk_ui::{SearchResultItem, SearchResults};
use rune::Tx;

use super::Shell;
use super::search::{Found, Paging};
use crate::timeline::TimelineKey;

/// How long the text rests before it is searched, in seconds.
pub(crate) const RUN_AFTER_S: f64 = 0.3;
/// Results within this many of the end ask for the next page.
const PAGE_AHEAD_ROWS: usize = 10;

impl Shell {
    /// The field's text changed: it is searched once it rests.
    pub(crate) fn search_edited(&mut self, tx: &mut Tx) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        search.highlight = 0;
        let due_s = tx.clock() + RUN_AFTER_S;
        search.run_due_s = Some(due_s);
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
        let answered = matches!(search.found, Found::Ready(_) | Found::Loading);
        if answered && search.asked.as_ref() == Some(&query) {
            return;
        }
        search.stop();
        search.asked = Some(query.clone());
        search.found = Found::Loading;
        search.opened = None;
        let (account, revision) = (search.account, search.revision);
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let request = client
                .session(account)
                .map_err(SearchError::from)
                .and_then(|session| session.search(query));
            let answer = match request {
                Ok(request) => {
                    let page = request.page(None).await;
                    page.map(|page| (request, page))
                }
                Err(error) => Err(error),
            };
            updates.send(move |shell| shell.search_answered(revision, answer));
        });
    }

    /// The first page of search `revision` arrived. Returns whether the
    /// view changed.
    fn search_answered(
        &mut self,
        revision: u64,
        answer: Result<(SearchRequestHandle, SearchPage), SearchError>,
    ) -> bool {
        let Some(search) = self
            .search
            .as_mut()
            .filter(|search| search.revision == revision)
        else {
            if let Ok((request, _)) = answer {
                request.cancel();
            }
            return false;
        };
        match answer {
            Ok((request, page)) => {
                search.found = Found::Ready(SearchResults::first(&page));
                search.request = Some(request);
            }
            // Replaced or stopped: what replaced it shows.
            Err(SearchError::Canceled) => return false,
            Err(error) => {
                log::warn!("a search failed: {error}");
                search.found = Found::Failed;
            }
        }
        true
    }

    /// Results `range` are drawn: near the end, the next page is asked.
    pub(crate) fn search_rows_visible(&mut self, range: Range<usize>) -> bool {
        let Some(search) = self.search.as_mut() else {
            return false;
        };
        let Found::Ready(results) = &search.found else {
            return false;
        };
        let near_end = range.end + PAGE_AHEAD_ROWS >= results.items().len();
        let (Some(cursor), Some(request)) = (results.next().cloned(), search.request.clone())
        else {
            return false;
        };
        if !near_end || search.paging == Paging::Loading {
            return false;
        }
        search.paging = Paging::Loading;
        let revision = search.revision;
        let Some(sdk) = &self.sdk else {
            return false;
        };
        let updates = sdk.updates();
        sdk.run(move |_client| async move {
            let page = request.page(Some(cursor)).await;
            updates.send(move |shell| shell.search_paged(revision, page));
        });
        true
    }

    /// The next page of search `revision` arrived.
    fn search_paged(&mut self, revision: u64, page: Result<SearchPage, SearchError>) -> bool {
        let Some(search) = self
            .search
            .as_mut()
            .filter(|search| search.revision == revision)
        else {
            return false;
        };
        let Found::Ready(results) = &mut search.found else {
            return false;
        };
        match page {
            Ok(page) => {
                results.append(&page);
                search.paging = Paging::Idle;
            }
            Err(SearchError::Canceled) => return false,
            Err(error) => {
                log::warn!("a page of search results failed: {error}");
                search.paging = Paging::Failed;
            }
        }
        true
    }

    /// "Try again" after a failure: the same text is searched again.
    pub(crate) fn search_retry(&mut self) {
        if let Some(search) = self.search.as_mut() {
            search.stop();
        }
        self.search_now();
    }

    /// A result was pressed, or Enter on it (`keyboard`): once the account
    /// confirms it (same connection, a channel it still reads), it opens
    /// in the conversation and the panel marks it. Results a reconnect made
    /// stale are searched again instead.
    pub(crate) fn open_hit(&mut self, id: Uuid, keyboard: bool) {
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let Found::Ready(results) = &search.found else {
            return;
        };
        let Some(item) = results.items().iter().find(|item| item.id() == id).cloned() else {
            return;
        };
        search.opened = Some(id);
        let (account, revision) = (search.account, search.revision);
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let target = client
                .session(account)
                .map_err(SearchError::from)
                .and_then(|session| session.search_target(&item.hit));
            updates.send_tx(move |shell, tx| {
                shell.hit_checked(revision, &item, target.is_ok(), keyboard, tx)
            });
        });
    }

    /// The account answered for result `item` of search `revision`.
    fn hit_checked(
        &mut self,
        revision: u64,
        item: &SearchResultItem,
        current: bool,
        keyboard: bool,
        tx: &mut Tx,
    ) -> bool {
        let Some(search) = self
            .search
            .as_mut()
            .filter(|search| search.revision == revision)
        else {
            return false;
        };
        if !current {
            // A reconnect since: the results no longer open; search again.
            search.stop();
            self.search_now();
            return true;
        }
        let key = TimelineKey {
            account: search.account,
            channel: item.channel,
        };
        self.open_message(key, item.id(), item.message.cursor(), keyboard, tx);
        true
    }
}
