//! Bounded resumable full-message hydration using actual history pages; never installs timeline coverage or read state.

use super::{SearchError, SearchHit, SearchTarget};
use crate::{Generation, HistoryCursor, HistoryQuery, Message, Session, Uuid};
use std::{collections::VecDeque, sync::Arc};
use tokio_util::sync::CancellationToken;

pub(crate) struct DetachedStore {
    slots: Arc<tokio::sync::Semaphore>,
    entries: tokio::sync::Mutex<VecDeque<(Generation, Uuid, Arc<Message>)>>,
}
impl Default for DetachedStore {
    fn default() -> Self {
        Self {
            slots: Arc::new(tokio::sync::Semaphore::new(2)),
            entries: tokio::sync::Mutex::new(VecDeque::new()),
        }
    }
}
/// One detached full-message hydration turn. Pending work can resume using its request handle.
#[derive(Clone)]
pub enum DetachedMessageUpdate {
    /// Full backend message, without canonical history coverage.
    Ready(Arc<Message>),
    /// Four pages examined; another explicitly requested turn can continue.
    Pending,
    /// Backend history ended without this message, e.g. it was deleted.
    Unavailable,
}
struct Position {
    cursor: Option<HistoryCursor>,
    finished: bool,
}
/// Resumable original-message lookup. The pinned backend has no message-ID/context endpoint.
/// At most four real pages/turn, two concurrent hydrations/account and 32 retained messages/account.
pub struct DetachedMessageRequest {
    session: Session,
    target: SearchTarget,
    position: tokio::sync::Mutex<Position>,
    cancel: CancellationToken,
}
impl Session {
    /// Starts detached hydration without opening or changing a timeline window.
    /// # Errors
    /// Returns foreign/stale result or unavailable original channel.
    pub fn hydrate_search_hit(
        &self,
        hit: &SearchHit,
    ) -> Result<DetachedMessageRequest, SearchError> {
        let target = self.search_target(hit)?;
        Ok(DetachedMessageRequest {
            session: self.clone(),
            target,
            position: tokio::sync::Mutex::new(Position {
                cursor: None,
                finished: false,
            }),
            cancel: self.inner.epochs.current().cancel.child_token(),
        })
    }
}
impl DetachedMessageRequest {
    /// Cancels this hydration without canceling its search query or session.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }
    /// Advances one bounded turn, preserving backend cursors verbatim across Pending results.
    /// # Errors
    /// Returns canceled/stale account or malformed/backend history response.
    pub async fn next_batch(&self) -> Result<DetachedMessageUpdate, SearchError> {
        let session = self.session.clone();
        let target = self.target.clone();
        let cancel = self.cancel.clone();
        let mut position = self.position.lock().await;
        let initial = position.cursor.clone();
        let finished = position.finished;
        if finished {
            return Ok(DetachedMessageUpdate::Unavailable);
        }
        let guard = CancelOnDrop(CancellationToken::new());
        let dropped = guard.0.clone();
        let (update,cursor)=self.session.client.dispatch(async move {
            let work=fetch(&session,&target,initial,&cancel);
            Ok(tokio::select!{biased;()=dropped.cancelled()=>Err(SearchError::Canceled),result=work=>result})
        }).await??;
        self.session.inner.epochs.validate(self.target.generation)?;
        position.cursor = cursor;
        position.finished = !matches!(update, DetachedMessageUpdate::Pending);
        Ok(update)
    }
}
struct CancelOnDrop(CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}
impl Drop for DetachedMessageRequest {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
async fn fetch(
    session: &Session,
    target: &SearchTarget,
    mut cursor: Option<HistoryCursor>,
    cancel: &CancellationToken,
) -> Result<(DetachedMessageUpdate, Option<HistoryCursor>), SearchError> {
    let work = async {
        let store = &session.inner.detached;
        let _slot = Arc::clone(&store.slots)
            .acquire_owned()
            .await
            .map_err(|_| crate::ClientError::Closed)?;
        session.inner.epochs.validate(target.generation)?;
        {
            let mut entries = store.entries.lock().await;
            entries.retain(|(generation, _, _)| *generation == target.generation);
            if let Some(index) = entries.iter().position(|(_, id, _)| *id == target.message) {
                let entry = entries.remove(index).ok_or(SearchError::InvalidData)?;
                let message = Arc::clone(&entry.2);
                entries.push_back(entry);
                return Ok((DetachedMessageUpdate::Ready(message), cursor));
            }
        }
        let epoch = session.inner.epochs.current();
        let rest = crate::scoped_request::authenticated(session, &epoch).await?;
        for _ in 0..4 {
            let page = session
                .client
                .inner
                .request(rest.messages(
                    target.channel,
                    &HistoryQuery {
                        cursor: cursor.as_ref(),
                        limit: Some(100),
                        ..Default::default()
                    },
                ))
                .await?;
            session.inner.epochs.validate(target.generation)?;
            if page.channel_id.as_deref().and_then(|id| id.parse().ok()) != Some(target.channel) {
                return Err(SearchError::InvalidData);
            }
            let messages = page.messages.unwrap_or_default();
            if messages.len() > 100 {
                return Err(SearchError::InvalidData);
            }
            validate_page(target, cursor.as_ref(), &messages)?;
            if let Some(message) = messages.iter().find(|message| {
                message.id.as_deref().and_then(|id| id.parse().ok()) == Some(target.message)
            }) {
                let message = Arc::new(message.clone());
                let mut entries = store.entries.lock().await;
                session.inner.epochs.validate(target.generation)?;
                entries.retain(|(_, id, _)| *id != target.message);
                entries.push_back((target.generation, target.message, Arc::clone(&message)));
                if entries.len() > 32 {
                    entries.pop_front();
                }
                return Ok((DetachedMessageUpdate::Ready(message), cursor));
            }
            if page.has_more != Some(true) {
                return Ok((DetachedMessageUpdate::Unavailable, cursor));
            }
            let next = messages
                .last()
                .and_then(Message::cursor)
                .ok_or(SearchError::InvalidData)?;
            if cursor.as_ref() == Some(&next) {
                return Err(SearchError::InvalidData);
            }
            cursor = Some(next);
        }
        Ok((DetachedMessageUpdate::Pending, cursor))
    };
    tokio::select! {biased;()=cancel.cancelled()=>Err(SearchError::Canceled),result=work=>result}
}

fn validate_page(
    target: &SearchTarget,
    cursor: Option<&HistoryCursor>,
    messages: &[Message],
) -> Result<(), SearchError> {
    let key = |cursor: &HistoryCursor| {
        super::cursor_key(&super::SearchCursor {
            created_at: cursor.created_at.clone(),
            id: cursor.id.clone(),
        })
    };
    let mut previous = cursor.map(key).transpose()?;
    for message in messages {
        if message.channel_id.as_deref().and_then(|id| id.parse().ok()) != Some(target.channel) {
            return Err(SearchError::InvalidData);
        }
        let cursor = message.cursor().ok_or(SearchError::InvalidData)?;
        let current = key(&cursor)?;
        if previous.is_some_and(|previous| current >= previous) {
            return Err(SearchError::InvalidData);
        }
        if current.1 == target.message
            && current
                != super::cursor_key(&super::SearchCursor {
                    created_at: target.created_at.clone(),
                    id: target.message.to_string(),
                })?
        {
            return Err(SearchError::InvalidData);
        }
        previous = Some(current);
    }
    Ok(())
}
