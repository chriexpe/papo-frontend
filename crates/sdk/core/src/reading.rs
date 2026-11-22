//! Account-owned ordered read state; history owns coverage and UI owns viewport/window selection.

pub(crate) mod catch_up_ops;
mod catch_up_state;
pub(crate) mod catch_up_sync;
mod counts;
pub(crate) mod evidence;
mod observers;
pub(crate) mod position_publish;
pub(crate) mod positions;
pub(crate) mod reconcile;
pub(crate) mod reconcile_marks;
#[cfg(test)]
mod tests;
mod upload;

pub use observers::ReadingObserver;

use crate::{
    CatchUpSnapshot, CatchUpView, ClientError, Generation, HistoryError, HistorySnapshot,
    MessageKey, ReadingPosition, ReadingPositionsSnapshot, Session, StorageError,
    VisibilityContinuity,
    catch_up::{CatchUpStore, VisibleBounds},
};
use papo_api::Uuid;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{Mutex, watch};

/// Ordered unread count. Incomplete coverage never claims an exact total.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum UnreadCount {
    /// Complete cached coverage from the read point through the known live head.
    Exact(u64),
    /// This many cached messages are unread; unknown messages may also exist.
    AtLeast(u64),
}
impl Default for UnreadCount {
    fn default() -> Self {
        Self::AtLeast(0)
    }
}

/// One channel's durable cursors and cache-derived unread projection.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadState {
    /// Furthest locally reported visible message, committed with synchronous=FULL.
    pub local_point: Option<MessageKey>,
    /// Furthest resolved server mark (including confirmed uploads).
    pub uploaded_point: Option<MessageKey>,
    /// Unread messages after the greater of local and uploaded points.
    pub unread_count: UnreadCount,
    /// First cached unread message, not a claim that an unknown prefix is complete.
    pub first_unread: Option<Uuid>,
}
impl ReadState {
    pub(super) fn point(&self) -> Option<MessageKey> {
        self.local_point.max(self.uploaded_point)
    }
    pub(super) fn pending(&self) -> Option<MessageKey> {
        self.local_point
            .filter(|point| Some(*point) > self.uploaded_point)
    }
}

/// Account-scoped reading image; bounded watch subscribers may coalesce intermediate versions.
#[derive(Clone, Debug)]
pub struct ReadingSnapshot {
    /// Current session generation.
    pub generation: Generation,
    /// Monotonic publication version within this session.
    pub version: u64,
    /// Reading state keyed by backend channel UUID, never by UI selection.
    pub channels: BTreeMap<Uuid, ReadState>,
}

/// Sanitized read reporting/persistence failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReadingError {
    /// A supplied message lacks a valid microsecond-order cursor.
    #[error("invalid read point")]
    InvalidPoint,
    /// A report named an ID outside its validated chronological snapshot.
    #[error("invalid visible report")]
    InvalidReport,
    /// History closure, version or generation validation failed.
    #[error(transparent)]
    History(#[from] HistoryError),
    /// Durable read state could not be committed.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Session lifecycle or transport failed.
    #[error(transparent)]
    Session(#[from] ClientError),
}

impl From<crate::InvalidMessageKey> for ReadingError {
    fn from(_: crate::InvalidMessageKey) -> Self {
        Self::InvalidPoint
    }
}

pub(crate) type ObserverVisibilityMap = BTreeMap<u64, BTreeMap<Uuid, VisibleBounds>>;

pub(crate) struct Reading {
    pub serial: Mutex<()>,
    pub image: Arc<crate::observable::StateCell<Arc<ReadingSnapshot>>>,
    pub catch_up_states: std::sync::Mutex<CatchUpStore>,
    pub catch_up_image: Arc<crate::observable::StateCell<Arc<crate::CatchUpSnapshot>>>,
    pub position_image: Arc<crate::observable::StateCell<Arc<crate::ReadingPositionsSnapshot>>>,
    pub started: std::sync::atomic::AtomicBool,
    pub uploads: Mutex<BTreeMap<Uuid, Arc<tokio::sync::Notify>>>,
    pub heads: Mutex<BTreeMap<Uuid, MessageKey>>,
    pub unresolved: Mutex<BTreeMap<Uuid, crate::ServerReadPoint>>,
    pub connection: Mutex<Option<(Generation, crate::reconcile::ResourceConnection)>>,
    pub observers: std::sync::Mutex<ObserverVisibilityMap>,
    pub next_observer: std::sync::atomic::AtomicU64,
    pub reconcile_keys: std::sync::Mutex<BTreeMap<Uuid, catch_up_sync::ReconcileKey>>,
    pub positions: Arc<positions::PositionCoordinator>,
}
impl Reading {
    pub fn new(generation: Generation) -> Self {
        Self {
            serial: Mutex::new(()),
            image: crate::observable::StateCell::new(Arc::new(ReadingSnapshot {
                generation,
                version: 0,
                channels: BTreeMap::new(),
            })),
            catch_up_states: std::sync::Mutex::new(CatchUpStore::default()),
            catch_up_image: crate::observable::StateCell::new(Arc::new(CatchUpSnapshot {
                generation,
                version: 0,
                channels: BTreeMap::new(),
            })),
            position_image: crate::observable::StateCell::new(Arc::new(ReadingPositionsSnapshot {
                generation,
                version: 0,
                channels: BTreeMap::new(),
            })),
            started: std::sync::atomic::AtomicBool::new(false),
            uploads: Mutex::new(BTreeMap::new()),
            heads: Mutex::new(BTreeMap::new()),
            unresolved: Mutex::new(BTreeMap::new()),
            connection: Mutex::new(None),
            observers: std::sync::Mutex::new(BTreeMap::new()),
            next_observer: std::sync::atomic::AtomicU64::new(1),
            reconcile_keys: std::sync::Mutex::new(BTreeMap::new()),
            positions: Arc::new(positions::PositionCoordinator::default()),
        }
    }

    pub fn close(&self) {
        self.image.close();
        self.catch_up_image.close();
        self.position_image.close();
        self.positions.close();
    }

    pub async fn publish(
        &self,
        session: &crate::session::SessionInner,
        mut image: ReadingSnapshot,
    ) -> Result<(), ClientError> {
        // Resource and backend denials install the position fences under R1. Preserve
        // those fences even if this image was built before the denial's durable cleanup.
        let admissions = session.history.admission.read().await;
        session.epochs.apply(image.generation, || {
            self.positions
                .restrict_reading(&mut image, &self.image.borrow(), |id| {
                    admissions.channel(id)
                        == Some(crate::history::access::ChannelAdmission::Allowed)
                });
            self.publish_current(image)
        })?
    }

    /// Clear denied read state without taking the reading writer's async mutex.
    /// A rebuild may already own that mutex while it waits for a backend readback.
    pub(super) async fn remove_revoked(
        &self,
        session: &crate::session::SessionInner,
        generation: Generation,
        channels: &[Uuid],
    ) -> Result<(), ClientError> {
        let admissions = session.history.admission.read().await;
        session.epochs.apply(generation, || {
            let current = self.image.borrow().clone();
            let mut next = None;
            for &id in channels {
                if current.channels.contains_key(&id)
                    && !matches!(
                        admissions.channel(id),
                        Some(
                            crate::history::access::ChannelAdmission::Allowed
                                | crate::history::access::ChannelAdmission::Quarantined
                        )
                    )
                {
                    next.get_or_insert_with(|| (*current).clone())
                        .channels
                        .remove(&id);
                }
            }
            if let Some(mut image) = next {
                image.generation = generation;
                self.publish_current(image)?;
            }
            Ok(())
        })?
    }

    /// Caller holds the generation fence while installing all inputs for this image.
    fn publish_current(&self, mut image: ReadingSnapshot) -> Result<(), ClientError> {
        image.version = self
            .image
            .borrow()
            .version
            .checked_add(1)
            .ok_or(ClientError::Closed)?;
        self.image.send_replace(Arc::new(image));
        Ok(())
    }
}

impl Session {
    /// Atomically subscribes to durable reading state. Coalesces intermediate images and
    /// closes when this account is removed, replaced, signed out or shut down.
    #[must_use]
    pub fn subscribe_reading(&self) -> watch::Receiver<Arc<ReadingSnapshot>> {
        self.inner.reading.image.subscribe()
    }

    /// Returns this account's last published state for one channel, if known and session is active.
    #[must_use]
    pub fn reading_state(&self, channel: Uuid) -> Option<ReadState> {
        if self.is_closed() {
            return None;
        }
        self.inner
            .reading
            .image
            .borrow()
            .channels
            .get(&channel)
            .cloned()
    }

    /// Creates a per-window visibility observer isolating scroll continuity across concurrent windows.
    #[must_use]
    pub fn reading_observer(&self) -> ReadingObserver {
        ReadingObserver::new(self.clone())
    }

    /// Atomically subscribes to catch-up revision watermarks across accessible channels.
    /// Resolve detailed boundaries via `Session::catch_up_view`. Closes on sign-out, removal,
    /// replacement or shutdown.
    #[must_use]
    pub fn subscribe_catch_up(&self) -> watch::Receiver<Arc<CatchUpSnapshot>> {
        self.inner.reading.catch_up_image.subscribe()
    }

    /// Resolves catch-up markers and start-target readiness against `snapshot`, returning `None`
    /// when `channel` has no active catch-up interval or all skipped foreign messages are read.
    /// # Errors
    /// Returns `ReadingError::InvalidPoint` if a cached message lacks valid chronological metadata.
    pub fn catch_up_view(
        &self,
        channel: Uuid,
        snapshot: &HistorySnapshot,
    ) -> Result<Option<CatchUpView>, ReadingError> {
        catch_up_ops::resolve_view(self, channel, snapshot)
    }

    /// Atomically subscribes to durable saved reading positions for all accessible channels.
    #[must_use]
    pub fn subscribe_reading_positions(&self) -> watch::Receiver<Arc<ReadingPositionsSnapshot>> {
        self.inner.reading.position_image.subscribe()
    }

    /// Returns the last committed reading position for `channel`, if one exists and session is active.
    #[must_use]
    pub fn reading_position(&self, channel: Uuid) -> Option<ReadingPosition> {
        if self.is_closed() {
            return None;
        }
        self.inner
            .reading
            .position_image
            .borrow()
            .channels
            .get(&channel)
            .copied()
    }

    /// Durably saves or clears a client reading position (`offset_units` relative to `message_id`).
    /// Coalesces rapid bursts and acknowledges only after a `synchronous=FULL` commit.
    /// # Errors
    /// Returns storage errors, session closure/supersession or history closure when revoked.
    pub async fn save_reading_position(
        &self,
        channel: Uuid,
        position: Option<ReadingPosition>,
    ) -> Result<(), ReadingError> {
        let session = self.clone();
        let token = self.inner.tasks.token();
        self.client
            .dispatch(async move {
                let _token = token;
                let generation = session.generation();
                Ok(session
                    .inner
                    .reading
                    .positions
                    .save(
                        &session.client,
                        &session.inner,
                        channel,
                        generation,
                        position,
                    )
                    .await)
            })
            .await
            .map_err(ReadingError::from)?
    }

    /// Deliberately clears the active catch-up interval for `channel` with a `synchronous=FULL` commit.
    /// # Errors
    /// Returns storage, generation, revocation or session errors.
    pub async fn clear_catch_up(&self, channel: Uuid) -> Result<(), ReadingError> {
        let session = self.clone();
        let token = self.inner.tasks.token();
        self.client
            .dispatch(async move {
                let _token = token;
                Ok(catch_up_ops::clear_catch_up_inner(&session, channel).await)
            })
            .await
            .map_err(ReadingError::from)?
    }

    pub(super) async fn report_with_observer(
        &self,
        observer_id: u64,
        channel: Uuid,
        generation: Generation,
        history_version: u64,
        ids: &[Uuid],
        continuity: VisibilityContinuity,
    ) -> Result<(), ReadingError> {
        let session = self.clone();
        let ids = ids.to_vec();
        let token = self.inner.tasks.token();
        self.client
            .dispatch(async move {
                let _token = token;
                Ok(catch_up_ops::report_current(
                    &session,
                    observer_id,
                    channel,
                    generation,
                    history_version,
                    &ids,
                    continuity,
                )
                .await)
            })
            .await
            .map_err(ReadingError::from)?
    }
}
