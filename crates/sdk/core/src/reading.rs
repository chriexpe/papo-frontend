//! Account-owned ordered read state; history owns coverage and UI owns viewport/window selection.

mod counts;
pub(crate) mod reconcile;
#[cfg(test)]
mod tests;
mod upload;

use crate::{ClientError, Generation, HistoryError, Session, StorageError};
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

pub(crate) struct Reading {
    pub serial: Mutex<()>,
    pub image: Arc<crate::observable::StateCell<Arc<ReadingSnapshot>>>,
    pub started: std::sync::atomic::AtomicBool,
    pub uploads: Mutex<BTreeMap<Uuid, Arc<tokio::sync::Notify>>>,
    pub heads: Mutex<BTreeMap<Uuid, ReadPoint>>,
    pub unresolved: Mutex<BTreeMap<Uuid, crate::ServerReadPoint>>,
    pub connection: Mutex<Option<(Generation, crate::reconcile::ResourceConnection)>>,
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
            started: std::sync::atomic::AtomicBool::new(false),
            uploads: Mutex::new(BTreeMap::new()),
            heads: Mutex::new(BTreeMap::new()),
            unresolved: Mutex::new(BTreeMap::new()),
            connection: Mutex::new(None),
        }
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

    /// Reports only IDs actually visible in a canonical history snapshot. Callers also
    /// validate their own window version and only report while unobscured/foreground.
    /// An old generation/version or any detached/nonmember ID rejects the whole report.
    /// Success acknowledges the local point only after a synchronous=FULL commit;
    /// fetching or reaching a focused window's end never implicitly reports the live head.
    /// # Errors
    /// Returns history version/closure, invalid report, generation or durable storage errors.
    pub async fn report_visible(
        &self,
        channel: Uuid,
        generation: Generation,
        history_version: u64,
        ids: &[Uuid],
    ) -> Result<(), ReadingError> {
        let session = self.clone();
        let ids = ids.to_vec();
        let token = self.inner.tasks.token();
        self.client
            .dispatch(async move {
                let _token = token;
                Ok(session
                    .report_current(channel, generation, history_version, &ids)
                    .await)
            })
            .await
            .map_err(ReadingError::from)?
    }

    async fn report_current(
        &self,
        channel: Uuid,
        generation: Generation,
        history_version: u64,
        ids: &[Uuid],
    ) -> Result<(), ReadingError> {
        let _serial = self.inner.reading.serial.lock().await;
        self.inner.epochs.validate(generation)?;
        let history = self.history(channel).await?;
        let (snapshot, point) = history
            .visible_snapshot(generation, history_version, ids)
            .await?;
        let Some(point) = point else {
            return Ok(());
        };
        let mut image = (**self.inner.reading.image.borrow()).clone();
        image.generation = generation;
        let state = image.channels.entry(channel).or_default();
        if Some(point) <= state.local_point {
            return Ok(());
        }
        self.client
            .inner
            .registry
            .advance_visible_read(history, generation, history_version, point)
            .await?;
        self.inner.epochs.validate(generation)?;
        state.local_point = Some(point);
        counts::update(
            state,
            &snapshot,
            self.inner.reading.heads.lock().await.get(&channel).copied(),
        )?;
        if self
            .inner
            .reading
            .unresolved
            .lock()
            .await
            .contains_key(&channel)
        {
            counts::unresolved(state);
        }
        self.inner.reading.publish(&self.inner, image)?;
        upload::schedule(self, channel).await;
        Ok(())
    }
}
