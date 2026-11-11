//! History-handle lifecycle and cache-first admission; fetching and reconnect replay live separately.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use papo_api::{HistoryOrder, HistoryQuery, RestClient, Uuid};
use tokio::sync::{Mutex, RwLock, watch};

use crate::{AccountCache, ClientError, ConnectionState, Generated, Generation, Session};

use super::{
    HistoryClosedReason, HistoryError, HistoryPage, HistorySource, merge::Ticket, ordering::Point,
    persistence::State, publication::Publication,
};

#[derive(Clone)]
pub(super) struct CacheConnection {
    pub cache: Arc<AccountCache>,
    pub rest: RestClient,
    pub generation: Generation,
}

pub(crate) struct HistoryStore {
    pub(super) changes: watch::Sender<()>,
    pub(super) channels: Mutex<BTreeMap<Uuid, Arc<Channel>>>,
    pub(super) verified: RwLock<BTreeSet<Uuid>>,
    pub(super) connection: RwLock<Option<CacheConnection>>,
}

impl Default for HistoryStore {
    fn default() -> Self {
        Self {
            channels: Mutex::default(),
            verified: RwLock::default(),
            connection: RwLock::default(),
            changes: watch::channel(()).0,
        }
    }
}

pub(super) struct Channel {
    pub id: Uuid,
    pub state: Mutex<State>,
    pub opens: AtomicUsize,
    pub pins_serial: Mutex<()>,
    pub(super) publication: std::sync::Mutex<Publication>,
    pub(super) changes: watch::Sender<()>,
}

impl Channel {
    pub fn new(id: Uuid, changes: watch::Sender<()>) -> Self {
        Self {
            id,
            state: Mutex::new(State::default()),
            opens: AtomicUsize::new(0),
            pins_serial: Mutex::new(()),
            publication: std::sync::Mutex::default(),
            changes,
        }
    }

    pub fn close(&self, reason: HistoryClosedReason) {
        self.publication
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .close(reason);
        self.changes.send_replace(());
    }

    pub fn check(&self) -> Result<(), HistoryError> {
        self.publication
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .check()
    }
}

/// Revocable raw-history access to one account/channel. Clones share data, not pagination cursors.
/// A caller can independently page from the newest edge or any exact server cursor.
pub struct HistoryHandle {
    pub(super) session: Session,
    pub(super) channel: Arc<Channel>,
}

impl Clone for HistoryHandle {
    fn clone(&self) -> Self {
        self.channel.opens.fetch_add(1, Ordering::Relaxed);
        Self {
            session: self.session.clone(),
            channel: Arc::clone(&self.channel),
        }
    }
}

impl Drop for HistoryHandle {
    fn drop(&mut self) {
        self.channel.opens.fetch_sub(1, Ordering::Relaxed);
    }
}

impl Session {
    /// Open this account's history after a fresh resource reconciliation verified the channel.
    /// Opening restores only this channel's cache; it never authorizes cached permissions.
    /// # Errors
    /// Returns unverified/revoked channel, stale generation or sanitized cache failures.
    pub async fn history(&self, channel: Uuid) -> Result<HistoryHandle, HistoryError> {
        let session = self.clone();
        self.client
            .dispatch(async move {
                let result = async {
                    let epoch = session.inner.epochs.current();
                    session.inner.epochs.validate(epoch.generation)?;
                    let store = &session.inner.history;
                    if !store.verified.read().await.contains(&channel) {
                        return Err(HistoryError::Unverified);
                    }
                    let connection = store
                        .connection
                        .read()
                        .await
                        .clone()
                        .ok_or(HistoryError::Unverified)?;
                    let channel = {
                        let mut channels = store.channels.lock().await;
                        Arc::clone(channels.entry(channel).or_insert_with(|| {
                            Arc::new(Channel::new(channel, store.changes.clone()))
                        }))
                    };
                    channel.check()?;
                    let mut state = channel.state.lock().await;
                    state
                        .ensure(&connection.cache, *channel.id.as_bytes(), epoch.generation)
                        .await?;
                    session.inner.epochs.validate(epoch.generation)?;
                    channel.check()?;
                    session.inner.epochs.apply(epoch.generation, || {
                        channel.initialize(&state, epoch.generation)
                    })??;
                    drop(state);
                    channel.opens.fetch_add(1, Ordering::Relaxed);
                    Ok(HistoryHandle {
                        session: session.clone(),
                        channel,
                    })
                }
                .await;
                Ok(result)
            })
            .await
            .map_err(HistoryError::from)?
    }
}

impl HistoryHandle {
    /// The owning account. Account replacement never retargets an existing handle.
    #[must_use]
    pub fn account(&self) -> crate::AccountId {
        self.session.account()
    }

    /// The backend channel UUID.
    #[must_use]
    pub fn channel(&self) -> Uuid {
        self.channel.id
    }

    /// Read a bounded page in either direction, preferring proven cache coverage. Offline
    /// results may be stale and stop at explicit gaps; they never bridge unknown ranges.
    /// `None` with descending order starts at newest; ascending starts at the oldest edge.
    /// # Errors
    /// Returns typed closure, malformed-page, generation, transport or storage failures.
    pub async fn page(&self, query: HistoryQuery<'_>) -> Result<HistoryPage, HistoryError> {
        let cursor = query.cursor.map(Point::from);
        let order = query.order;
        let limit = usize::from(query.limit.unwrap_or(30));
        if limit == 0 || limit > 100 {
            return Err(HistoryError::InvalidPage);
        }
        if let Some(cursor) = &cursor {
            cursor.key()?;
        }
        let handle = self.clone();
        let token = self.session.inner.tasks.token();
        self.session.client.dispatch(async move {
            let _token = token;
            let epoch = handle.session.inner.epochs.current();
            let result = tokio::select! {
                biased;
                () = handle.session.inner.root.cancelled() => Err(handle.account_closed()),
                () = epoch.cancel.cancelled() => Err(HistoryError::from(ClientError::Superseded)),
                result = handle.page_current(cursor, order, limit, epoch.generation) => result,
            };
            Ok(result)
        }).await.map_err(HistoryError::from)?
    }

    /// Read the separate pin snapshot without making detached pins chronological members.
    /// The newest-page fetch and reconnect refresh this snapshot from the pinned list.
    /// # Errors
    /// Returns revoked, unverified, stale-generation or cache failures.
    pub async fn pins(&self) -> Result<Generated<Vec<Arc<papo_api::wire::Message>>>, HistoryError> {
        let handle = self.clone();
        let token = self.session.inner.tasks.token();
        self.session
            .client
            .dispatch(async move {
                let _token = token;
                let epoch = handle.session.inner.epochs.current();
                let work = async {
                    let generation = epoch.generation;
                    let connection = handle.connection().await?;
                    if connection.generation == generation
                        && matches!(
                            handle.session.inner.connection.borrow().state,
                            ConnectionState::Online
                        )
                    {
                        super::fetch::pins(&handle, &connection, generation).await?;
                    }
                    let mut state = handle.channel.state.lock().await;
                    handle.restore(&mut state, &connection, generation).await?;
                    handle.check(generation)?;
                    let value = state
                        .model
                        .entries
                        .values()
                        .filter(|entry| entry.pinned && !entry.deleted)
                        .filter_map(|entry| entry.message.clone().map(Arc::new))
                        .collect();
                    Ok(Generated { generation, value })
                };
                let result = tokio::select! {
                    biased;
                    () = handle.session.inner.root.cancelled() => Err(handle.account_closed()),
                    () = epoch.cancel.cancelled() => Err(HistoryError::from(ClientError::Superseded)),
                    result = work => result,
                };
                Ok(result)
            })
            .await
            .map_err(HistoryError::from)?
    }

    pub(super) fn check(&self, generation: Generation) -> Result<(), HistoryError> {
        self.channel.check()?;
        if self.session.is_closed() {
            return Err(self.account_closed());
        }
        self.session.inner.epochs.validate(generation)?;
        Ok(())
    }

    fn account_closed(&self) -> HistoryError {
        self.channel.close(HistoryClosedReason::AccountClosed);
        self.channel
            .check()
            .err()
            .unwrap_or(HistoryError::Closed(HistoryClosedReason::AccountClosed))
    }

    /// Restore and validate this channel's cache incarnation under the caller's state lock.
    /// A revocation that raced the caller is reported as the handle's typed closure, never as
    /// the masked cache row it left behind.
    async fn restore(
        &self,
        state: &mut State,
        connection: &CacheConnection,
        generation: Generation,
    ) -> Result<(), HistoryError> {
        let result = async {
            state
                .ensure(&connection.cache, *self.channel.id.as_bytes(), generation)
                .await?;
            connection
                .cache
                .history_validate(
                    *self.channel.id.as_bytes(),
                    state.set_id.ok_or(HistoryError::Unverified)?,
                    generation,
                )
                .await?;
            self.session
                .inner
                .epochs
                .apply(generation, || self.channel.initialize(state, generation))??;
            Ok(())
        }
        .await;
        result.map_err(|error| self.check(generation).err().unwrap_or(error))
    }

    pub(super) async fn connection(&self) -> Result<CacheConnection, HistoryError> {
        self.channel.check()?;
        self.session
            .inner
            .history
            .connection
            .read()
            .await
            .clone()
            .ok_or(HistoryError::Unverified)
    }

    async fn page_current(
        &self,
        cursor: Option<Point>,
        order: HistoryOrder,
        limit: usize,
        generation: Generation,
    ) -> Result<HistoryPage, HistoryError> {
        self.check(generation)?;
        let connection = self.connection().await?;
        let ticket = {
            let mut state = self.channel.state.lock().await;
            self.restore(&mut state, &connection, generation).await?;
            self.check(generation)?;
            let cached = state
                .model
                .page(cursor.as_ref(), order, limit, generation.get())?;
            let online = connection.generation == generation
                && matches!(
                    self.session.inner.connection.borrow().state,
                    ConnectionState::Online
                );
            if !online || cached.fresh {
                return Ok(HistoryPage {
                    generation,
                    next_cursor: cached
                        .messages
                        .last()
                        .and_then(papo_api::wire::Message::cursor)
                        .or_else(|| cached.gap.as_ref().map(|gap| gap.cursor.clone())),
                    messages: cached.messages.into_iter().map(Arc::new).collect(),
                    source: HistorySource::Cache,
                    stale: !cached.fresh,
                    has_more: cached.has_more,
                    gap: cached.gap,
                });
            }
            Ticket {
                channel: *self.channel.id.as_bytes(),
                generation: generation.get(),
                revision: state.model.revision,
                cursor,
                order,
                limit,
            }
        };
        super::fetch::page(self, &connection, ticket).await
    }
}
