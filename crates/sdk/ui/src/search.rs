//! Account-bound observable search pages; core owns grammar/backend requests and clients own navigation/Back state.

use papo_sdk::{
    SearchCursor, SearchError, SearchHit, SearchPage, SearchQuery, SearchRequestHandle,
    SearchTarget, Session,
};
use std::sync::{Arc, Mutex};
use tokio::sync::watch;

/// Complete detached page state; never contains or changes the active timeline window.
#[derive(Clone)]
#[non_exhaustive]
pub enum SearchUpdate {
    /// No accepted query yet.
    Idle,
    /// Request is in progress; previous accepted page remains caller-owned.
    Loading,
    /// One bounded backend page.
    Ready(Arc<SearchPage>),
    /// Query failed; error detail remains on the direct method result.
    Fault,
    /// Account revoked.
    Closed,
}
struct State {
    request: Option<SearchRequestHandle>,
    revision: u64,
}
/// One independent search surface. Separate instances can query without canceling each other.
#[derive(Clone)]
pub struct SearchService {
    session: Session,
    state: Arc<Mutex<State>>,
    updates: watch::Sender<SearchUpdate>,
    loads: Arc<tokio::sync::Mutex<()>>,
}
impl SearchService {
    /// Creates an independent observable search surface for one account.
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self {
            session,
            state: Arc::new(Mutex::new(State {
                request: None,
                revision: 0,
            })),
            updates: watch::channel(SearchUpdate::Idle).0,
            loads: Arc::new(tokio::sync::Mutex::new(())),
        }
    }
    /// Atomic current page plus capacity-one updates. No profile/roster fetch occurs.
    #[must_use]
    pub fn subscribe(&self) -> SearchSubscription {
        let mut subscription = SearchSubscription {
            baseline: SearchUpdate::Idle,
            session: self.session.clone(),
            state: Arc::clone(&self.state),
            updates: self.updates.subscribe(),
            connection: self.session.subscribe_connection(),
        };
        subscription.baseline = subscription.current();
        subscription
    }
    /// Replaces the active query, immediately canceling any older request.
    /// # Errors
    /// Returns invalid query, closed account or stale response; old pages never replace new ones.
    pub async fn search(&self, query: SearchQuery) -> Result<SearchPage, SearchError> {
        let request = self.session.search(query)?;
        let revision = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(old) = state.request.replace(request.clone()) {
                old.cancel();
            }
            state.revision = state.revision.checked_add(1).ok_or(SearchError::Canceled)?;
            state.revision
        };
        self.load(request, revision, None).await
    }
    /// Fetches the next exact backend cursor for the active query. Returned pages remain detached.
    /// # Errors
    /// Returns missing/canceled query, stale account or backend failure.
    pub async fn page(&self, cursor: SearchCursor) -> Result<SearchPage, SearchError> {
        let (request, revision) = {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                state.request.clone().ok_or(SearchError::Canceled)?,
                state.revision,
            )
        };
        self.load(request, revision, Some(cursor)).await
    }
    /// Cancels this surface's query without affecting other search instances.
    pub fn cancel(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.revision = state.revision.saturating_add(1);
        if let Some(request) = state.request.take() {
            request.cancel();
        }
        self.updates.send_replace(SearchUpdate::Idle);
    }
    /// Revalidates the result's original account/channel; clients keep query/results for Back.
    /// # Errors
    /// Rejects stale/foreign identity. Does not fetch history or mark read.
    pub fn target(&self, hit: &SearchHit) -> Result<SearchTarget, SearchError> {
        self.session.search_target(hit)
    }
    async fn load(
        &self,
        request: SearchRequestHandle,
        revision: u64,
        cursor: Option<SearchCursor>,
    ) -> Result<SearchPage, SearchError> {
        let _load = self.loads.lock().await;
        {
            let state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.revision != revision || state.request.is_none() {
                return Err(SearchError::Canceled);
            }
            self.updates.send_replace(SearchUpdate::Loading);
        }
        let result = request.page(cursor).await;
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.revision != revision || state.request.is_none() {
            return Err(SearchError::Canceled);
        }
        let update = match &result {
            Ok(page) => SearchUpdate::Ready(Arc::new(page.clone())),
            Err(_) if self.session.is_closed() => SearchUpdate::Closed,
            Err(_) => SearchUpdate::Fault,
        };
        self.updates.send_replace(update);
        result
    }
}

/// Coalesced search pages with account/generation invalidation even while no query is running.
pub struct SearchSubscription {
    /// Initial accepted state, validated at subscription.
    pub baseline: SearchUpdate,
    session: Session,
    state: Arc<Mutex<State>>,
    updates: watch::Receiver<SearchUpdate>,
    connection: watch::Receiver<papo_sdk::ConnectionStatus>,
}
impl SearchSubscription {
    /// Returns the current view only when its source account/generation is still valid.
    #[must_use]
    pub fn current(&self) -> SearchUpdate {
        if self.session.is_closed() {
            return SearchUpdate::Closed;
        }
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state
            .request
            .as_ref()
            .is_some_and(|request| request.generation() != self.session.generation())
        {
            if let Some(request) = &state.request {
                request.cancel();
            }
            return SearchUpdate::Idle;
        }
        self.updates.borrow().clone()
    }
    /// Waits for a page or lifecycle invalidation; Closed remains sticky after logout/removal.
    pub async fn next(&mut self) -> SearchUpdate {
        if self.session.is_closed() {
            return SearchUpdate::Closed;
        }
        tokio::select! {
            result = self.connection.changed() => if result.is_err() {return SearchUpdate::Closed;},
            result = self.updates.changed() => if result.is_err() {return SearchUpdate::Closed;},
        }
        self.current()
    }
}
