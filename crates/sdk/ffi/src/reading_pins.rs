//! Standalone channel pin lists and durable saved reading-position access across FFI.
//! Viewport continuity and catch-up boundaries remain in `timeline.rs`.

use std::sync::Arc;

use papo_sdk::{ReadingPosition, ReadingPositionsSnapshot};

use crate::{
    client::FfiClient,
    errors::FfiClientError,
    ids::{FfiAccountId, parse_uuid},
    messages::FfiMessage,
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription, watch_stream},
    timeline_errors::{FfiHistoryClosedReason, FfiHistoryError, FfiReadingError},
};

/// Durable client-defined reading scroll anchor stored verbatim in SQLite.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiReadingPosition {
    /// Anchor message UUID string.
    pub message_id: String,
    /// Client-defined signed scroll offset relative to `message_id`.
    pub offset_units: i64,
}

impl From<ReadingPosition> for FfiReadingPosition {
    fn from(v: ReadingPosition) -> Self {
        Self {
            message_id: v.message_id.to_string(),
            offset_units: v.offset_units,
        }
    }
}

/// Channel entry inside `FfiReadingPositionsSnapshot`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiChannelReadingPosition {
    /// Channel or DM UUID string.
    pub channel_id: String,
    /// Saved reading anchor.
    pub position: FfiReadingPosition,
}

/// Generation-scoped map of durable saved reading positions for one account.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiReadingPositionsSnapshot {
    /// Connection epoch.
    pub generation: u64,
    /// Publication version.
    pub version: u64,
    /// Channels holding saved reading positions.
    pub positions: Vec<FfiChannelReadingPosition>,
}

impl From<&ReadingPositionsSnapshot> for FfiReadingPositionsSnapshot {
    fn from(s: &ReadingPositionsSnapshot) -> Self {
        Self {
            generation: s.generation.get(),
            version: s.version,
            positions: s
                .channels
                .iter()
                .map(|(ch, pos)| FfiChannelReadingPosition {
                    channel_id: ch.to_string(),
                    position: (*pos).into(),
                })
                .collect(),
        }
    }
}

/// Listener for durable reading position changes on an account.
#[uniffi::export(with_foreign)]
pub trait FfiReadingPositionsListener: Send + Sync {
    /// Initial atomic baseline.
    fn on_snapshot(&self, snapshot: FfiReadingPositionsSnapshot);
    /// Subsequent updates.
    fn on_update(&self, snapshot: FfiReadingPositionsSnapshot);
    /// Terminal notification on account revocation.
    fn on_closed(&self);
}

/// Pinned messages response tagged with validating connection generation.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiPinnedMessages {
    /// Session connection epoch.
    pub generation: u64,
    /// Pinned messages in authoritative backend order.
    pub messages: Vec<FfiMessage>,
}

#[uniffi::export]
impl FfiClient {
    /// Reads durable saved reading position for `(account, channel_id)` without starting network work.
    ///
    /// # Errors
    /// Returns invalid argument or session revocation errors.
    pub fn reading_position(
        &self,
        account: FfiAccountId,
        channel_id: String,
    ) -> Result<Option<FfiReadingPosition>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let channel = parse_uuid(&channel_id)?;
        Ok(session.reading_position(channel).map(Into::into))
    }

    /// Durably saves or clears reading position with `synchronous=FULL` commit.
    ///
    /// # Errors
    /// Returns [`FfiReadingError`].
    pub async fn save_reading_position(
        &self,
        account: FfiAccountId,
        channel_id: String,
        position: Option<FfiReadingPosition>,
    ) -> Result<(), FfiReadingError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiReadingError::Session { error: err })?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiReadingError::InvalidPoint)?;
        let sdk_pos = position
            .map(|p| {
                Ok::<_, FfiReadingError>(ReadingPosition {
                    message_id: parse_uuid(&p.message_id)
                        .map_err(|_| FfiReadingError::InvalidPoint)?,
                    offset_units: p.offset_units,
                })
            })
            .transpose()?;
        self.run_scoped(
            || FfiReadingError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                session
                    .save_reading_position(channel, sdk_pos)
                    .await
                    .map_err(Into::into)
            },
        )
        .await
    }

    /// Subscribes to durable reading position updates across an account's channels.
    ///
    /// # Errors
    /// Returns missing session or worker startup failures.
    pub fn subscribe_reading_positions(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiReadingPositionsListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiClientError> {
        let session = self.session_by_id(&account)?;
        let (baseline, stream) = watch_stream(session.subscribe_reading_positions());
        let l_snap = Arc::clone(&listener);
        let l_upd = Arc::clone(&listener);
        let l_end = listener;
        spawn_stream_subscription(
            &self.runtime,
            &self.tasks,
            &self.shutdown,
            self.callbacks.clone(),
            FfiReadingPositionsSnapshot::from(baseline.as_ref()),
            stream,
            move |snap| {
                l_snap.on_snapshot(snap);
                true
            },
            move |snap| {
                l_upd.on_update(FfiReadingPositionsSnapshot::from(snap.as_ref()));
                true
            },
            move || l_end.on_closed(),
        )
    }

    /// Fetches authoritative pinned messages for `(account, channel_id)` over REST and updates cache.
    ///
    /// # Errors
    /// Returns [`FfiHistoryError`].
    pub async fn pinned_messages(
        &self,
        account: FfiAccountId,
        channel_id: String,
    ) -> Result<FfiPinnedMessages, FfiHistoryError> {
        let session = self
            .session_by_id(&account)
            .map_err(|_| FfiHistoryError::Closed {
                reason: FfiHistoryClosedReason::AccountClosed,
            })?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiHistoryError::InvalidPage)?;
        self.run_scoped(
            || FfiHistoryError::Closed {
                reason: FfiHistoryClosedReason::AccountClosed,
            },
            async move {
                let history = session.history(channel).await?;
                let pins = history.pins().await?;
                Ok(FfiPinnedMessages {
                    generation: pins.generation.get(),
                    messages: pins.value.iter().map(|m| m.as_ref().into()).collect(),
                })
            },
        )
        .await
    }

    /// Reads previously cached pinned messages for `(account, channel_id)` without network access.
    ///
    /// # Errors
    /// Returns [`FfiHistoryError`] including permission closure when channel is no longer readable.
    pub async fn cached_pinned_messages(
        &self,
        account: FfiAccountId,
        channel_id: String,
    ) -> Result<FfiPinnedMessages, FfiHistoryError> {
        let session = self
            .session_by_id(&account)
            .map_err(|_| FfiHistoryError::Closed {
                reason: FfiHistoryClosedReason::AccountClosed,
            })?;
        let channel = parse_uuid(&channel_id).map_err(|_| FfiHistoryError::InvalidPage)?;
        self.run_scoped(
            || FfiHistoryError::Closed {
                reason: FfiHistoryClosedReason::AccountClosed,
            },
            async move {
                let history = session.history(channel).await?;
                let pins = history.cached_pins().await?;
                Ok(FfiPinnedMessages {
                    generation: pins.generation.get(),
                    messages: pins.value.iter().map(|m| m.as_ref().into()).collect(),
                })
            },
        )
        .await
    }
}
