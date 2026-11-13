//! Detached authoritative search requests and stable navigation identities; never mutates canonical history or reads.

mod detached;
pub(crate) use detached::DetachedStore;
pub use detached::{DetachedMessageRequest, DetachedMessageUpdate};
mod grammar;
pub use grammar::{SearchNames, SearchQuery};
#[cfg(test)]
mod tests;

use crate::{AccountId, ClientError, Generation, HistoryOrder, Session, Uuid};
use papo_api::{
    Cursor,
    wire::{SearchRequest, SearchResult},
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Query, identity, cancellation or backend failures without server content in diagnostics.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SearchError {
    /// Invalid query or malformed backend result/cursor.
    #[error("invalid search data")]
    InvalidData,
    /// Query explicitly canceled or replaced.
    #[error("search canceled")]
    Canceled,
    /// Account authentication, transport or generation failure.
    #[error(transparent)]
    Session(#[from] ClientError),
}

/// Backend tuple, retained verbatim rather than synthesized from UUID order alone.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchCursor {
    /// Exact backend timestamp.
    pub created_at: String,
    /// Backend message UUID spelling.
    pub id: String,
}

/// Account-bound detached result. Possession never grants canonical history coverage.
#[derive(Clone)]
pub struct SearchHit {
    account: AccountId,
    generation: Generation,
    /// Authoritative detached REST fields; media hydration is a separate explicit request.
    pub result: Arc<SearchResult>,
}

/// Validated source identity for client-owned search navigation and Back restoration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchTarget {
    /// Account that produced this result.
    pub account: AccountId,
    /// Request generation, revalidated when opening.
    pub generation: Generation,
    /// Original channel, never the active DM inferred from UI state.
    pub channel: Uuid,
    /// Original message.
    pub message: Uuid,
    /// Exact original timestamp, usable with focused history cursors.
    pub created_at: String,
}

/// One bounded backend page.
#[derive(Clone)]
pub struct SearchPage {
    /// Account that issued this page, including an empty result page.
    pub account: AccountId,
    /// Request generation, validated before returning.
    pub generation: Generation,
    /// Backend-ordered detached records, at most 100.
    pub hits: Vec<SearchHit>,
    /// Last result's exact tuple only when backend `has_more` is true.
    pub next: Option<SearchCursor>,
}

/// One independent cancelable search. Dropping all clones cancels pending work.
#[derive(Clone)]
pub struct SearchRequestHandle {
    session: Session,
    request: SearchRequest,
    cancel: CancellationToken,
    _lifetime: Arc<CancelOnDrop>,
    generation: Generation,
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

impl Session {
    /// Starts an independent detached query. No current channel is implicitly inherited.
    /// # Errors
    /// Returns invalid query, account closure or stale generation.
    pub fn search(&self, query: SearchQuery) -> Result<SearchRequestHandle, SearchError> {
        let request = query.into_request()?;
        let epoch = self.inner.epochs.current();
        self.inner.epochs.validate(epoch.generation)?;
        let cancel = epoch.cancel.child_token();
        Ok(SearchRequestHandle {
            session: self.clone(),
            request,
            generation: epoch.generation,
            _lifetime: Arc::new(CancelOnDrop(cancel.clone())),
            cancel,
        })
    }
    /// Revalidates a detached result's original account/channel without acknowledging a read.
    /// # Errors
    /// Rejects foreign account, stale generation or missing current resource membership.
    pub fn search_target(&self, hit: &SearchHit) -> Result<SearchTarget, SearchError> {
        if self.account() != hit.account {
            return Err(SearchError::InvalidData);
        }
        self.inner.epochs.validate(hit.generation)?;
        let result = &hit.result;
        let channel = parse_id(result.channel_id.as_deref())?;
        let resources = self.inner.resources.borrow();
        if resources.generation != hit.generation
            || !resources
                .channels
                .iter()
                .any(|c| c.id.as_deref().and_then(|s| s.parse().ok()) == Some(channel))
                && !resources
                    .dms
                    .iter()
                    .any(|d| d.id.parse().ok() == Some(channel))
        {
            return Err(SearchError::InvalidData);
        }
        Ok(SearchTarget {
            account: hit.account,
            generation: hit.generation,
            channel,
            message: parse_id(result.id.as_deref())?,
            created_at: result.created_at.clone().ok_or(SearchError::InvalidData)?,
        })
    }
}
impl SearchRequestHandle {
    /// Connection generation captured when the query was created.
    #[must_use]
    pub fn generation(&self) -> Generation {
        self.generation
    }

    /// Cancels all clones of this query; later queries are unaffected.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    /// Fetches an exact backend page, independent of canonical timeline and viewport state.
    /// # Errors
    /// Returns canceled, stale, invalid backend data or transport errors.
    pub async fn page(&self, cursor: Option<SearchCursor>) -> Result<SearchPage, SearchError> {
        let this = self.clone();
        let session = self.session.clone();
        let attempt = CancelOnDrop(CancellationToken::new());
        let dropped = attempt.0.clone();
        self.session
            .client
            .dispatch(async move {
                Ok(tokio::select! {
                    biased;
                    () = dropped.cancelled() => Err(SearchError::Canceled),
                    result = this.fetch(cursor) => result,
                })
            })
            .await
            .map_err(SearchError::from)
            .and_then(|r| {
                session.inner.epochs.validate(self.generation)?;
                if self.cancel.is_cancelled() {
                    return Err(SearchError::Canceled);
                }
                r
            })
    }
    async fn fetch(&self, cursor: Option<SearchCursor>) -> Result<SearchPage, SearchError> {
        let epoch = self.session.inner.epochs.current();
        self.session.inner.epochs.validate(self.generation)?;
        let work = async {
            let rest = crate::scoped_request::authenticated(&self.session, &epoch).await?;
            let order = if self.request.order.as_deref() == Some("asc") {
                HistoryOrder::Asc
            } else {
                HistoryOrder::Desc
            };
            let boundary = cursor.as_ref().map(|c| Cursor {
                since: &c.created_at,
                last_id: Some(&c.id),
                order: Some(order),
            });
            let response = self
                .session
                .client
                .inner
                .request(rest.search(&self.request, boundary.as_ref()))
                .await?;
            self.session.inner.epochs.validate(self.generation)?;
            let results = response.results.unwrap_or_default();
            if results.len() > 100 {
                return Err(SearchError::InvalidData);
            }
            let mut previous = cursor.as_ref().map(cursor_key).transpose()?;
            for result in &results {
                if result.r#type.as_deref() != Some("message") {
                    return Err(SearchError::InvalidData);
                }
                parse_id(result.channel_id.as_deref())?;
                let key = cursor_key(&result_cursor(result)?)?;
                if previous.is_some_and(|prior| {
                    if order == HistoryOrder::Asc {
                        key <= prior
                    } else {
                        key >= prior
                    }
                }) {
                    return Err(SearchError::InvalidData);
                }
                previous = Some(key);
            }
            let next = if response.has_more == Some(true) {
                let next = result_cursor(results.last().ok_or(SearchError::InvalidData)?)?;
                Some(next)
            } else {
                None
            };
            Ok(SearchPage {
                account: self.session.account(),
                generation: self.generation,
                hits: results
                    .into_iter()
                    .map(|result| SearchHit {
                        account: self.session.account(),
                        generation: self.generation,
                        result: Arc::new(result),
                    })
                    .collect(),
                next,
            })
        };
        tokio::select! { biased;
            () = self.cancel.cancelled() => Err(SearchError::Canceled),
            () = epoch.cancel.cancelled() => Err(ClientError::Superseded.into()),
            result = work => result,
        }
    }
}
fn parse_id(raw: Option<&str>) -> Result<Uuid, SearchError> {
    raw.and_then(|s| s.parse().ok())
        .ok_or(SearchError::InvalidData)
}
fn result_cursor(result: &SearchResult) -> Result<SearchCursor, SearchError> {
    Ok(SearchCursor {
        created_at: result.created_at.clone().ok_or(SearchError::InvalidData)?,
        id: result.id.clone().ok_or(SearchError::InvalidData)?,
    })
}
fn cursor_key(cursor: &SearchCursor) -> Result<(i64, Uuid), SearchError> {
    let time = chrono::DateTime::parse_from_rfc3339(&cursor.created_at)
        .map_err(|_| SearchError::InvalidData)?;
    Ok((time.timestamp_micros(), parse_id(Some(&cursor.id))?))
}
