//! Shared notification policy and FULL claims across live, push and background; platforms own banner presentation.

pub(crate) mod backend;
pub(crate) mod context;
pub(crate) mod inbox;
pub(crate) mod ledger;
pub(crate) mod live;
mod policy;
pub(crate) mod push;
mod triggers;
pub use inbox::{InboxCursor, InboxPage, NotificationRecord};
pub use policy::{ChannelNotificationMode, NotificationSettings};
pub use push::{PushDevice, PushRegistration};
#[cfg(all(test, target_os = "linux"))]
mod crash;
#[cfg(test)]
mod retention_tests;
#[cfg(test)]
mod tests;

use crate::{AccountId, ClientError, Generation, Session, Uuid};
use std::{
    collections::BTreeSet,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64},
    },
};
use tokio::sync::{mpsc, watch};

/// Independent notification failures; canonical history remains readable.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum NotificationError {
    /// Corrupt or incomplete backend/storage record.
    #[error("invalid notification data")]
    InvalidData,
    /// Notification producer or platform consumer exceeded its bounded queue.
    #[error("notification queue overflow")]
    Overflow,
    /// Authentication, generation, storage or transport failure.
    #[error(transparent)]
    Session(#[from] ClientError),
}
impl From<crate::StorageError> for NotificationError {
    fn from(error: crate::StorageError) -> Self {
        Self::Session(error.into())
    }
}
impl From<crate::storage_worker::WorkerError> for NotificationError {
    fn from(error: crate::storage_worker::WorkerError) -> Self {
        Self::from(crate::StorageError::from(error))
    }
}
impl From<rusqlite::Error> for NotificationError {
    fn from(error: rusqlite::Error) -> Self {
        Self::from(crate::StorageError::from(error))
    }
}

/// Observable state independent of the resource/history services.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotificationStatus {
    /// Account was revoked; no later work can deliver from it.
    Closed,
    /// No producer failure has been observed.
    Ready,
    /// A producer failed; refreshing inbox remains independently available.
    Fault,
}

/// Account-scoped identity validated again immediately before client navigation/presentation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationTarget {
    /// Verified source account.
    pub account: AccountId,
    /// Connection that admitted the candidate.
    pub generation: Generation,
    /// Original channel identity.
    pub channel: Uuid,
    /// Canonical backend message, never a notification row ID.
    pub message: Uuid,
}

/// Claimed at-most-once platform handoff. Clients translate generic/private presentation.
#[derive(Clone)]
pub struct NotificationDelivery {
    /// Scoped identity; call `validate_target` before presentation/tap navigation.
    pub target: NotificationTarget,
    /// Preview is absent unless account and device policy explicitly allow it.
    pub preview: Option<String>,
    /// Whether sound is permitted by the shared policy.
    pub sound: bool,
}

pub(crate) struct Notifications {
    pub push_closing: AtomicBool,
    pub inbox_revision: AtomicU64,
    pub backend: Mutex<Option<backend::BackendPolicy>>,
    pub canonical_points: tokio::sync::Mutex<context::CanonicalPoints>,
    pub poll_serial: tokio::sync::Mutex<()>,
    pub serial: tokio::sync::Mutex<()>,
    pub context: Mutex<BTreeSet<Uuid>>,
    pub deliveries: mpsc::Sender<NotificationDelivery>,
    pub receiver: tokio::sync::Mutex<mpsc::Receiver<NotificationDelivery>>,
    pub events: Mutex<Option<mpsc::Sender<crate::Generated<papo_api::gateway::Event>>>>,
    pub status: watch::Sender<NotificationStatus>,
}
impl Default for Notifications {
    fn default() -> Self {
        let (deliveries, receiver) = mpsc::channel(64);
        Self {
            push_closing: AtomicBool::new(false),
            inbox_revision: AtomicU64::new(0),
            backend: Mutex::default(),
            canonical_points: tokio::sync::Mutex::default(),
            poll_serial: tokio::sync::Mutex::new(()),
            serial: tokio::sync::Mutex::new(()),
            context: Mutex::default(),
            deliveries,
            receiver: tokio::sync::Mutex::new(receiver),
            events: Mutex::default(),
            status: watch::channel(NotificationStatus::Ready).0,
        }
    }
}

/// One account policy/inbox/claim capability. Clones share a single consuming delivery queue.
#[derive(Clone)]
pub struct NotificationService {
    pub(crate) session: Session,
}
impl Session {
    /// Gets shared notification policy and authoritative inbox operations.
    #[must_use]
    pub fn notifications(&self) -> NotificationService {
        NotificationService {
            session: self.clone(),
        }
    }
}
impl NotificationService {
    /// Subscribes to independent producer faults; no message content enters status diagnostics.
    #[must_use]
    pub fn subscribe_status(&self) -> watch::Receiver<NotificationStatus> {
        self.session.inner.notifications.status.subscribe()
    }
    /// Supplies currently visible channels, without navigation/widget state. Empty means no suppression.
    /// # Errors
    /// Rejects revoked account handles.
    pub fn set_visible_channels(&self, channels: BTreeSet<Uuid>) -> Result<(), NotificationError> {
        self.session
            .inner
            .epochs
            .apply(self.session.generation(), || {
                *self
                    .session
                    .inner
                    .notifications
                    .context
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = channels;
            })?;
        Ok(())
    }
    /// Consumes one already committed delivery. Multiple consumers compete, never broadcast duplicates.
    /// # Errors
    /// Returns revoked account; stale queued generations are silently discarded.
    pub async fn next_delivery(&self) -> Result<NotificationDelivery, NotificationError> {
        let session = &self.session;
        let mut receiver = session.inner.notifications.receiver.lock().await;
        loop {
            let delivery = tokio::select! { biased;
                () = session.inner.root.cancelled() => return Err(ClientError::Closed.into()),
                value = receiver.recv() => value.ok_or(ClientError::Closed)?,
            };
            if session
                .inner
                .epochs
                .validate(delivery.target.generation)
                .is_ok()
            {
                return Ok(delivery);
            }
        }
    }

    /// Validates account and generation before a notification handoff or tap; never marks read.
    /// # Errors
    /// Returns foreign identity, missing channel or stale/revoked session.
    pub fn validate_target(&self, target: &NotificationTarget) -> Result<(), NotificationError> {
        if target.account != self.session.account() {
            return Err(NotificationError::InvalidData);
        }
        self.session.inner.epochs.validate(target.generation)?;
        let resources = self.session.inner.resources.borrow();
        if resources.generation != target.generation
            || !policy::contains_channel(&resources, target.channel)
        {
            return Err(NotificationError::InvalidData);
        }
        Ok(())
    }
    /// Loads account-local device policy; backend channel/account settings are applied in addition.
    /// # Errors
    /// Returns corrupt state, revoked account or durable failure.
    pub async fn settings(&self) -> Result<NotificationSettings, NotificationError> {
        let session = self.session.clone();
        self.session
            .client
            .dispatch(async move { Ok(ledger::settings(&session).await) })
            .await?
    }
    /// Persists device policy with FULL before returning. All producers consult this same state.
    /// # Errors
    /// Returns revoked account or durable failure.
    pub async fn set_settings(
        &self,
        settings: NotificationSettings,
    ) -> Result<(), NotificationError> {
        let session = self.session.clone();
        self.session
            .client
            .dispatch(async move {
                let _serial = session.inner.notifications.serial.lock().await;
                Ok(ledger::save_settings(&session, settings).await)
            })
            .await?
    }
}

pub(super) fn id(raw: Option<&str>) -> Result<Uuid, NotificationError> {
    raw.and_then(|id| id.parse().ok())
        .ok_or(NotificationError::InvalidData)
}
pub(super) fn now_us() -> Result<i64, NotificationError> {
    let micros = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| NotificationError::InvalidData)?
        .as_micros();
    i64::try_from(micros).map_err(|_| NotificationError::InvalidData)
}
