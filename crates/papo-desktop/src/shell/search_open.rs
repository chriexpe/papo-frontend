//! The search panel's results from the shell's side (`docs/design.md`
//! §10, Search): the scope choice, the next page as the end of the results
//! comes near (a failed page waits for its Try again row, never retrying
//! on its own), and opening a result: once the account confirms it, it
//! opens in the conversation (`message_open`) and the panel marks it;
//! results a reconnect made stale are searched again instead.

use std::ops::Range;

use papo_sdk::{ClientError, Generation, SearchError, SearchPage, Uuid};
use papo_sdk_ui::{SearchResultItem, SearchResults};
use rune::Tx;

use super::Shell;
use super::search::{Found, Paging, SEARCH_FIELD_KEY, Scope};
use crate::timeline::TimelineKey;

/// Results within this many of the end ask for the next page.
const PAGE_AHEAD_ROWS: usize = 10;

/// What the account said about a result about to open.
#[derive(Clone, Copy)]
enum HitCheck {
    /// Same connection, a channel it still reads: it opens.
    Current,
    /// A reconnect since: the results are stale.
    Stale,
    /// Its channel is no longer readable, or it is malformed: it stays.
    Gone,
}

impl Shell {
    /// The scope choice changed: the same text searches there.
    pub(crate) fn search_scope_picked(&mut self, scope: Scope, tx: &mut Tx) {
        if let Some(search) = self.search.as_mut() {
            search.scope = scope;
        }
        self.search_now();
        tx.focus(SEARCH_FIELD_KEY, true);
    }

    /// The shown panel's results, when it has some.
    pub(crate) fn shown_results(&self) -> Option<&SearchResults> {
        match &self.shown_search()?.found {
            Found::Ready(results) => Some(results),
            _ => None,
        }
    }

    /// "This server ⌄": the scope menu grows down from it.
    pub(crate) fn open_scope_menu(&mut self, tx: &mut Tx) {
        use crate::menu::{MenuAction, MenuItem, OpenMenu, Placement};
        let key = super::search_panel::SCOPE_KEY;
        let items = vec![
            MenuItem::plain(
                "scope-server",
                crate::strings::search_scope_server(),
                MenuAction::SearchScope { channel: false },
                false,
            ),
            MenuItem::plain(
                "scope-channel",
                crate::strings::search_scope_channel(),
                MenuAction::SearchScope { channel: true },
                false,
            ),
        ];
        self.nav.menu = Some(OpenMenu::new(
            Placement::BelowEnd(key.to_owned()),
            key.to_owned(),
            items,
        ));
        tx.focus(crate::menu::MENU_KEY, true);
    }

    /// ↓ in the field with no list under it: the first result takes the
    /// keyboard. Returns whether there was one.
    pub(crate) fn focus_first_hit(&mut self, tx: &mut Tx) -> bool {
        let Some(first) = self
            .shown_results()
            .and_then(|results| results.items().first())
        else {
            return false;
        };
        tx.focus(super::search::hit_row_key(first.id()), true);
        true
    }

    /// Results `range` are drawn: near the end, the next page is asked,
    /// one at a time, and not after a failure (its Try again row asks).
    pub(crate) fn search_rows_visible(&mut self, range: Range<usize>) -> bool {
        let Some(search) = self.search.as_ref() else {
            return false;
        };
        let Found::Ready(results) = &search.found else {
            return false;
        };
        let near_end = range.end + PAGE_AHEAD_ROWS >= results.items().len();
        if !near_end || search.paging != Paging::Idle || search.loading() {
            return false;
        }
        self.search_next_page()
    }

    /// The failed page's Try again.
    pub(crate) fn search_page_retry(&mut self) {
        if let Some(search) = self.search.as_mut()
            && search.paging == Paging::Failed
        {
            search.paging = Paging::Idle;
            self.search_next_page();
        }
    }

    /// Asks the shown results' next page. Returns whether it was asked.
    fn search_next_page(&mut self) -> bool {
        let Some(search) = self.search.as_mut() else {
            return false;
        };
        let Found::Ready(results) = &search.found else {
            return false;
        };
        let (Some(cursor), Some(request)) = (results.next().cloned(), search.request.clone())
        else {
            return false;
        };
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

    /// A page the date window trimmed to nothing: no row can come near to
    /// ask for the next, so it is asked at once.
    pub(super) fn search_more_if_trimmed(&mut self) {
        let wanted = self.search.as_ref().is_some_and(
            |search| matches!(&search.found, Found::Ready(results) if results.wants_next()),
        );
        if wanted {
            self.search_next_page();
        }
    }

    /// The next page of search `revision` arrived.
    pub(super) fn search_paged(
        &mut self,
        revision: u64,
        page: Result<SearchPage, SearchError>,
    ) -> bool {
        let Some(search) = self.current_search(revision) else {
            return false;
        };
        let Found::Ready(results) = &mut search.found else {
            return false;
        };
        match page {
            Ok(page) => {
                results.append(&page);
                search.paging = Paging::Idle;
                self.search_more_if_trimmed();
            }
            // A reconnect since: the results are stale; search again.
            Err(SearchError::Session(ClientError::Superseded)) => {
                search.stop();
                self.search_now();
            }
            Err(error) => {
                log::warn!("a page of search results failed: {error}");
                search.paging = Paging::Failed;
            }
        }
        true
    }

    /// A result was pressed, or Enter on it (`keyboard`): the account is
    /// asked about it first.
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
        let (account, revision, generation) = (search.account, search.revision, search.generation);
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let check = match client.session(account) {
                Ok(session) => check_hit(&session, &item, generation),
                Err(_) => HitCheck::Gone,
            };
            updates
                .send_tx(move |shell, tx| shell.hit_checked(revision, &item, check, keyboard, tx));
        });
    }

    /// The account answered for result `item` of search `revision`: it
    /// opens if it is still the one the panel marks.
    fn hit_checked(
        &mut self,
        revision: u64,
        item: &SearchResultItem,
        check: HitCheck,
        keyboard: bool,
        tx: &mut Tx,
    ) -> bool {
        let Some(search) = self.current_search(revision) else {
            return false;
        };
        if search.opened != Some(item.id()) {
            return false;
        }
        match check {
            HitCheck::Current => {
                let key = TimelineKey {
                    account: search.account,
                    channel: item.channel,
                };
                self.open_message(key, item.id(), item.message.cursor(), keyboard, tx);
            }
            HitCheck::Stale => {
                search.stop();
                self.search_now();
            }
            HitCheck::Gone => {
                log::warn!("a search result is no longer readable");
                search.opened = None;
            }
        }
        true
    }
}

/// What `session` says about `item`, from results of connection
/// `generation`.
fn check_hit(
    session: &papo_sdk::Session,
    item: &SearchResultItem,
    generation: Option<Generation>,
) -> HitCheck {
    if generation.is_some_and(|generation| generation != session.generation()) {
        return HitCheck::Stale;
    }
    match session.search_target(&item.hit) {
        Ok(_) => HitCheck::Current,
        Err(SearchError::Session(ClientError::Superseded)) => HitCheck::Stale,
        Err(_) => HitCheck::Gone,
    }
}
