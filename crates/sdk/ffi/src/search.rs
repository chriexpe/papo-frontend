//! Detached message search, query parsing, local autocomplete matches, and message hydration across FFI.
//! Search requests never alter canonical timeline coverage or read positions.

use std::sync::Arc;

use futures_util::stream;
use papo_sdk::{DetachedMessageRequest, DetachedMessageUpdate};
use papo_sdk_ui::{SearchService, SearchUpdate};

use crate::{
    client::FfiClient,
    errors::FfiClientError,
    ids::FfiAccountId,
    operation_errors::FfiSearchError,
    search_records::{
        FfiDetachedMessageUpdate, FfiSearchCursor, FfiSearchHit, FfiSearchListener,
        FfiSearchMatches, FfiSearchPage, FfiSearchQuery, FfiSearchTarget, map_search_update,
    },
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription},
};

/// Cancellable detached message hydration request handle.
#[derive(uniffi::Object)]
pub struct FfiDetachedMessageRequest {
    /// Client runtime.
    client: Arc<FfiClient>,
    /// Underlying SDK detached hydration request.
    inner: Arc<DetachedMessageRequest>,
}

#[uniffi::export]
impl FfiDetachedMessageRequest {
    /// Cancels this hydration without canceling the owning search surface or session.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// Scans up to four bounded history pages for the full message payload.
    ///
    /// # Errors
    /// Returns [`FfiSearchError`].
    pub async fn next_batch(&self) -> Result<FfiDetachedMessageUpdate, FfiSearchError> {
        let req = Arc::clone(&self.inner);
        self.client
            .run_scoped(
                || FfiSearchError::Session {
                    error: FfiClientError::Closed,
                },
                async move {
                    let upd = req.next_batch().await?;
                    Ok(match upd {
                        DetachedMessageUpdate::Ready(msg) => FfiDetachedMessageUpdate::Ready {
                            message: msg.as_ref().into(),
                        },
                        DetachedMessageUpdate::Pending => FfiDetachedMessageUpdate::Pending,
                        DetachedMessageUpdate::Unavailable => FfiDetachedMessageUpdate::Unavailable,
                    })
                },
            )
            .await
    }
}

/// Independent search controller preserving cancellation isolation per UI screen/pane.
#[derive(uniffi::Object)]
pub struct FfiSearchSurface {
    /// Parent client runtime.
    client: Arc<FfiClient>,
    /// Underlying `papo-sdk-ui::SearchService`.
    inner: SearchService,
}

#[uniffi::export]
impl FfiSearchSurface {
    /// Subscribes to search surface transitions (`Idle`, `Loading`, `Ready`, `Fault`, `Closed`).
    ///
    /// # Errors
    /// Returns [`FfiClientError::Closed`] if closed or [`FfiClientError::Worker`] if thread spawn fails.
    pub fn subscribe(
        &self,
        listener: Arc<dyn FfiSearchListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        if self.client.is_closed() {
            return Err(FfiClientError::Closed);
        }
        let sub_owned = self.inner.subscribe();
        let baseline_sdk = sub_owned.current();
        if matches!(baseline_sdk, SearchUpdate::Closed) {
            return Err(FfiClientError::Closed);
        }
        let baseline = map_search_update(&baseline_sdk);
        let stream = Box::pin(stream::unfold(Some(sub_owned), |opt| async move {
            let mut s = opt?;
            let next = s.next().await;
            let ended = matches!(next, SearchUpdate::Closed);
            if ended {
                Some((next, None))
            } else {
                Some((next, Some(s)))
            }
        }));
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.client.runtime,
            &self.client.tasks,
            &self.client.shutdown,
            self.client.callbacks.clone(),
            baseline,
            stream,
            move |state| {
                l_snap.on_snapshot(state);
                true
            },
            move |upd| {
                l_upd.on_update(map_search_update(&upd));
                true
            },
            move || l_end.on_closed(),
        )
    }

    /// Parses human filter grammar (`from:`, `in:`, `has:`, `mentions:`) against current session names.
    ///
    /// # Errors
    /// Returns [`FfiSearchError::InvalidData`] or closure error.
    pub fn parse_query(&self, raw: String) -> Result<FfiSearchQuery, FfiSearchError> {
        self.inner
            .parse_query(&raw)
            .map(|q| FfiSearchQuery::from_sdk(&q))
            .map_err(Into::into)
    }

    /// Resolves matching local channel and member UUIDs for search autocomplete.
    #[must_use]
    pub fn local_matches(&self, needle: String) -> FfiSearchMatches {
        self.inner.local_matches(&needle).into()
    }

    /// Executes a typed search query, canceling any active request on this same surface.
    ///
    /// # Errors
    /// Returns [`FfiSearchError`].
    pub async fn search(&self, query: FfiSearchQuery) -> Result<FfiSearchPage, FfiSearchError> {
        let sdk_query = query.into_sdk()?;
        let service = self.inner.clone();
        self.client
            .run_scoped(
                || FfiSearchError::Session {
                    error: FfiClientError::Closed,
                },
                async move {
                    let page = service.search(sdk_query).await?;
                    Ok(FfiSearchPage {
                        account: FfiAccountId::from_sdk(page.account),
                        generation: page.generation.get(),
                        hits: page
                            .hits
                            .into_iter()
                            .map(|inner| Arc::new(FfiSearchHit { inner }))
                            .collect(),
                        next_cursor: page.next.as_ref().map(Into::into),
                    })
                },
            )
            .await
    }

    /// Loads next exact backend search page using previous page's `next_cursor`.
    ///
    /// # Errors
    /// Returns [`FfiSearchError`].
    pub async fn page(&self, cursor: FfiSearchCursor) -> Result<FfiSearchPage, FfiSearchError> {
        let service = self.inner.clone();
        self.client
            .run_scoped(
                || FfiSearchError::Session {
                    error: FfiClientError::Closed,
                },
                async move {
                    let page = service.page(cursor.into()).await?;
                    Ok(FfiSearchPage {
                        account: FfiAccountId::from_sdk(page.account),
                        generation: page.generation.get(),
                        hits: page
                            .hits
                            .into_iter()
                            .map(|inner| Arc::new(FfiSearchHit { inner }))
                            .collect(),
                        next_cursor: page.next.as_ref().map(Into::into),
                    })
                },
            )
            .await
    }

    /// Cancels the active query on this surface immediately and resets it to `Idle`.
    pub fn cancel(&self) {
        self.inner.cancel();
    }

    /// Revalidates original channel/message target before navigating to a focused timeline.
    ///
    /// # Errors
    /// Returns [`FfiSearchError`] on stale generation or deleted/inaccessible target.
    pub fn validate_hit_target(
        &self,
        hit: Arc<FfiSearchHit>,
    ) -> Result<FfiSearchTarget, FfiSearchError> {
        self.inner
            .target(&hit.inner)
            .map(Into::into)
            .map_err(Into::into)
    }

    /// Starts detached message hydration around `hit` without altering timeline windows.
    ///
    /// # Errors
    /// Returns [`FfiSearchError`].
    pub fn hydrate_hit(
        &self,
        hit: Arc<FfiSearchHit>,
    ) -> Result<Arc<FfiDetachedMessageRequest>, FfiSearchError> {
        let req = self.inner.hydrate(&hit.inner)?;
        Ok(Arc::new(FfiDetachedMessageRequest {
            client: Arc::clone(&self.client),
            inner: Arc::new(req),
        }))
    }
}

#[uniffi::export]
impl FfiClient {
    /// Creates an independent account-bound search surface (multiple views never cancel each other).
    ///
    /// # Errors
    /// Returns missing or closed session error.
    pub fn create_search_surface(
        self: &Arc<Self>,
        account: FfiAccountId,
    ) -> Result<Arc<FfiSearchSurface>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        Ok(Arc::new(FfiSearchSurface {
            client: Arc::clone(self),
            inner: SearchService::new(session),
        }))
    }
}
