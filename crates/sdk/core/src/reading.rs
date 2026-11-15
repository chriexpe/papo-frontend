//! Account-owned ordered read state; history owns coverage and UI owns viewport/window selection.

pub(crate) mod catch_up_ops;
pub(crate) mod catch_up_sync;
mod counts;
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
    CatchUpSnapshot, CatchUpState, CatchUpView, ClientError, Generation, HistoryError,
    HistorySnapshot, ReadingPosition, ReadingPositionsSnapshot, Session, StorageError,
    VisibilityContinuity,
};
use papo_api::{Uuid, wire::Message};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{Mutex, watch};

/// Message-order cursor, never an acknowledgement timestamp. Ordering is PostgreSQL
/// microseconds followed by the opaque UUID's bytes, exactly like history pagination.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct ReadPoint {
    /// Server message creation time in microseconds since Unix epoch.
    pub created_at_us: i64,
    /// Backend message UUID, used only as an equal-timestamp tiebreaker.
    #[serde(
        serialize_with = "crate::read_points::serialize_id",
        deserialize_with = "crate::read_points::deserialize_id"
    )]
    pub message_id: Uuid,
}
impl ReadPoint {
    pub(crate) fn message(message: &Message) -> Result<Self, ReadingError> {
        Self::parse(message.created_at.as_deref(), message.id.as_deref())
    }
    pub(crate) fn parse(created_at: Option<&str>, id: Option<&str>) -> Result<Self, ReadingError> {
        let at =
            chrono::DateTime::parse_from_rfc3339(created_at.ok_or(ReadingError::InvalidPoint)?)
                .map_err(|_| ReadingError::InvalidPoint)?;
        if at.timestamp_subsec_nanos() % 1_000 != 0 {
            return Err(ReadingError::InvalidPoint);
        }
        Ok(Self {
            created_at_us: at.timestamp_micros(),
            message_id: id
                .ok_or(ReadingError::InvalidPoint)?
                .parse()
                .map_err(|_| ReadingError::InvalidPoint)?,
        })
    }
}

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
    pub local_point: Option<ReadPoint>,
    /// Furthest resolved server mark (including confirmed uploads).
    pub uploaded_point: Option<ReadPoint>,
    /// Unread messages after the greater of local and uploaded points.
    pub unread_count: UnreadCount,
    /// First cached unread message, not a claim that an unknown prefix is complete.
    pub first_unread: Option<Uuid>,
}
impl ReadState {
    pub(super) fn point(&self) -> Option<ReadPoint> {
        self.local_point.max(self.uploaded_point)
    }
    pub(super) fn pending(&self) -> Option<ReadPoint> {
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

pub(crate) type ObserverVisibilityMap = BTreeMap<u64, BTreeMap<Uuid, (ReadPoint, ReadPoint)>>;
pub(crate) type AscSuccessorsState = (
    Generation,
    u64,
    BTreeMap<Uuid, crate::catch_up::AscSuccessors>,
);
pub(crate) type ReconcileKeyMap = BTreeMap<Uuid, (Generation, u64, u64, u64)>;

pub(crate) struct Reading {
    pub serial: Mutex<()>,
    pub image: Arc<crate::observable::StateCell<Arc<ReadingSnapshot>>>,
    pub catch_up_image: Arc<crate::observable::StateCell<Arc<crate::CatchUpSnapshot>>>,
    pub position_image: Arc<crate::observable::StateCell<Arc<crate::ReadingPositionsSnapshot>>>,
    pub started: std::sync::atomic::AtomicBool,
    pub uploads: Mutex<BTreeMap<Uuid, Arc<tokio::sync::Notify>>>,
    pub heads: Mutex<BTreeMap<Uuid, ReadPoint>>,
    pub unresolved: Mutex<BTreeMap<Uuid, crate::ServerReadPoint>>,
    pub connection: Mutex<Option<(Generation, crate::reconcile::ResourceConnection)>>,
    pub observers: std::sync::Mutex<ObserverVisibilityMap>,
    pub next_observer: std::sync::atomic::AtomicU64,
    pub asc_successors: std::sync::Mutex<Option<AscSuccessorsState>>,
    pub reconcile_keys: std::sync::Mutex<ReconcileKeyMap>,
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
            asc_successors: std::sync::Mutex::new(None),
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

    pub fn publish(
        &self,
        session: &crate::session::SessionInner,
        mut image: ReadingSnapshot,
    ) -> Result<(), ClientError> {
        session.epochs.apply(image.generation, || {
            image.version = self
                .image
                .borrow()
                .version
                .checked_add(1)
                .ok_or(ClientError::Closed)?;
            self.image.send_replace(Arc::new(image));
            Ok(())
        })?
    }
}

impl Session {
    /// Atomically subscribes to durable reading state. Coalesces intermediate images and
    /// closes when this account is removed, replaced, signed out or shut down.
    #[must_use]
    pub fn subscribe_reading(&self) -> watch::Receiver<Arc<ReadingSnapshot>> {
        self.inner.reading.image.subscribe()
    }

    /// Returns this account's last published state for one channel, if known.
    #[must_use]
    pub fn reading_state(&self, channel: Uuid) -> Option<ReadState> {
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

    /// Atomically subscribes to durable per-channel catch-up state.
    #[must_use]
    pub fn subscribe_catch_up(&self) -> watch::Receiver<Arc<CatchUpSnapshot>> {
        self.inner.reading.catch_up_image.subscribe()
    }

    /// Returns this account's last published catch-up state for one channel, if any.
    #[must_use]
    pub fn catch_up_state(&self, channel: Uuid) -> Option<CatchUpState> {
        self.inner
            .reading
            .catch_up_image
            .borrow()
            .channels
            .get(&channel)
            .cloned()
    }

    /// Resolves hourglass availability and display boundary IDs for `channel` against `snapshot`.
    /// # Errors
    /// Returns `ReadingError::InvalidPoint` if a cached message lacks valid chronological metadata.
    pub fn catch_up_view(
        &self,
        channel: Uuid,
        snapshot: &HistorySnapshot,
    ) -> Result<CatchUpView, ReadingError> {
        catch_up_ops::resolve_view(self, channel, snapshot)
    }

    /// Atomically subscribes to durable saved reading positions for all accessible channels.
    #[must_use]
    pub fn subscribe_reading_positions(&self) -> watch::Receiver<Arc<ReadingPositionsSnapshot>> {
        self.inner.reading.position_image.subscribe()
    }

    /// Returns the last committed reading position for `channel`, if one exists.
    #[must_use]
    pub fn reading_position(&self, channel: Uuid) -> Option<ReadingPosition> {
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

    /// Reports only IDs actually visible in a canonical history snapshot, paired with scroll continuity.
    /// Success acknowledges the local point and any catch-up interval transition only after a
    /// synchronous=FULL commit; reaching a focused window's end never implicitly reports the live head.
    /// # Errors
    /// Returns history version/closure, invalid report, generation or durable storage errors.
    pub async fn report_visible(
        &self,
        channel: Uuid,
        generation: Generation,
        history_version: u64,
        ids: &[Uuid],
        continuity: VisibilityContinuity,
    ) -> Result<(), ReadingError> {
        self.report_with_observer(0, channel, generation, history_version, ids, continuity)
            .await
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
