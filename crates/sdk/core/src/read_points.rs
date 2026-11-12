//! FULL-writer ordered read-point persistence; resolution, retries and unread policy belong to reading.

use crate::{
    AccountId, ClientError, Generation, Registry, StorageError, reading::ReadPoint,
    session::SessionInner,
};
use papo_api::Uuid;
use rusqlite::{OptionalExtension, params};
use std::{collections::BTreeMap, sync::Arc};

/// Backend read acknowledgement, not a message-order cursor.
/// `recorded_at` is the server's acknowledgement time and must never order messages.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ServerReadPoint {
    /// Backend UUID of the last read message.
    #[serde(serialize_with = "serialize_id", deserialize_with = "deserialize_id")]
    pub message_id: Uuid,
    /// Verbatim backend acknowledgement timestamp, when supplied.
    pub recorded_at: Option<String>,
}

pub(crate) fn serialize_id<S: serde::Serializer>(
    id: &Uuid,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&id.to_string())
}
pub(crate) fn deserialize_id<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Uuid, D::Error> {
    let value = <String as serde::Deserialize>::deserialize(deserializer)?;
    value
        .parse()
        .map_err(|_| serde::de::Error::custom("invalid read point UUID"))
}

/// Persisted server mark, compatible with the unresolved session-5b representation.
#[derive(Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct UploadedPoint {
    #[serde(flatten)]
    pub server: ServerReadPoint,
    #[serde(default)]
    pub created_at_us: Option<i64>,
    #[serde(default)]
    pub known_point: Option<ReadPoint>,
}
impl UploadedPoint {
    pub fn point(&self) -> Option<ReadPoint> {
        self.created_at_us
            .map(|created_at_us| ReadPoint {
                created_at_us,
                message_id: self.server.message_id,
            })
            .max(self.known_point)
    }
}

pub(crate) struct StoredPoints {
    pub local: Option<ReadPoint>,
    pub uploaded: Option<UploadedPoint>,
}

#[derive(Debug, thiserror::Error)]
enum ReadWriteError {
    #[error(transparent)]
    Session(#[from] ClientError),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error(transparent)]
    History(#[from] crate::HistoryError),
}
impl From<rusqlite::Error> for ReadWriteError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Storage(error.into())
    }
}
impl From<crate::storage_worker::WorkerError> for ReadWriteError {
    fn from(error: crate::storage_worker::WorkerError) -> Self {
        Self::Storage(error.into())
    }
}
impl From<ReadWriteError> for ClientError {
    fn from(error: ReadWriteError) -> Self {
        match error {
            ReadWriteError::Session(error) => error,
            ReadWriteError::Storage(error) => error.into(),
            ReadWriteError::History(error) => error.into(),
        }
    }
}
impl From<ReadWriteError> for crate::ReadingError {
    fn from(error: ReadWriteError) -> Self {
        match error {
            ReadWriteError::Session(error) => Self::Session(error),
            ReadWriteError::Storage(error) => Self::Storage(error),
            ReadWriteError::History(error) => Self::History(error),
        }
    }
}

impl Registry {
    pub(crate) async fn load_read_points(
        &self,
        account: AccountId,
    ) -> Result<BTreeMap<Uuid, StoredPoints>, StorageError> {
        let lease = Arc::clone(&self.lease);
        let rows = self.readers.query(self.path.clone(), move |tx| {
            let _lease = lease;
            tx.prepare("SELECT channel_id,local_point,uploaded_point FROM read_points WHERE server_id=?1 AND user_id=?2 AND EXISTS(SELECT 1 FROM accounts WHERE server_id=?1 AND user_id=?2 AND hidden=0)")?
                .query_map(params![account.server().as_bytes(), account.user().as_bytes()], |row| Ok((row.get::<_,Vec<u8>>(0)?, row.get::<_,Option<Vec<u8>>>(1)?, row.get::<_,Option<Vec<u8>>>(2)?)))?
                .collect::<Result<Vec<_>,_>>()
        }).await?;
        rows.into_iter()
            .map(|(channel, local, uploaded)| {
                let channel = Uuid::from_slice(&channel).map_err(|_| StorageError::InvalidState)?;
                let local = local
                    .map(|bytes| {
                        serde_json::from_slice(&bytes).map_err(|_| StorageError::InvalidState)
                    })
                    .transpose()?;
                let uploaded = uploaded
                    .map(|bytes| {
                        serde_json::from_slice(&bytes).map_err(|_| StorageError::InvalidState)
                    })
                    .transpose()?;
                Ok((channel, StoredPoints { local, uploaded }))
            })
            .collect()
    }

    pub(crate) async fn advance_visible_read(
        &self,
        history: crate::HistoryHandle,
        generation: Generation,
        version: u64,
        point: ReadPoint,
    ) -> Result<(), crate::ReadingError> {
        let bytes = serde_json::to_vec(&point).map_err(|_| StorageError::InvalidState)?;
        let account = history.account();
        let channel = history.channel();
        self.writer
            .write(move |tx| {
                history.with_current_snapshot(generation, version, || {
                    advance_local(tx, account, channel, point, &bytes)
                })?
            })
            .await
            .map_err(crate::ReadingError::from)
    }

    #[cfg(test)]
    pub(crate) async fn advance_local_read(
        &self,
        session: Arc<SessionInner>,
        channel: Uuid,
        generation: Generation,
        point: ReadPoint,
    ) -> Result<(), ClientError> {
        let bytes = serde_json::to_vec(&point).map_err(|_| StorageError::InvalidState)?;
        self.writer
            .write(move |tx| {
                session.epochs.apply(generation, || {
                    advance_local(tx, session.account, channel, point, &bytes)
                })?
            })
            .await
            .map_err(ClientError::from)
    }

    pub(crate) async fn save_uploaded_read(
        &self,
        session: Arc<SessionInner>,
        channel: Uuid,
        generation: Generation,
        uploaded: UploadedPoint,
    ) -> Result<(), ClientError> {
        let bytes = serde_json::to_vec(&uploaded).map_err(|_| StorageError::InvalidState)?;
        self.writer.write(move |tx| {
            session.epochs.apply(generation, || {
                let account = session.account;
                let changed = tx.execute("INSERT INTO read_points(server_id,user_id,channel_id,uploaded_point) SELECT ?1,?2,?3,?4 WHERE EXISTS(SELECT 1 FROM accounts WHERE server_id=?1 AND user_id=?2 AND hidden=0) ON CONFLICT(server_id,user_id,channel_id) DO UPDATE SET uploaded_point=excluded.uploaded_point",params![account.server().as_bytes(), account.user().as_bytes(), channel.as_bytes(), bytes])?;
                if changed != 1 { return Err(ReadWriteError::Session(ClientError::Closed)); }
                Ok::<_,ReadWriteError>(())
            })?
        }).await.map_err(ClientError::from)
    }
}

fn advance_local(
    tx: &rusqlite::Transaction<'_>,
    account: AccountId,
    channel: Uuid,
    point: ReadPoint,
    bytes: &[u8],
) -> Result<(), ReadWriteError> {
    let current: Option<Option<Vec<u8>>> = tx.query_row("SELECT local_point FROM read_points WHERE server_id=?1 AND user_id=?2 AND channel_id=?3", params![account.server().as_bytes(), account.user().as_bytes(), channel.as_bytes()], |row| row.get(0)).optional()?;
    let current: Option<ReadPoint> = current
        .flatten()
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| StorageError::InvalidState))
        .transpose()?;
    if current.is_some_and(|current| current >= point) {
        return Ok(());
    }
    let changed = tx.execute("INSERT INTO read_points(server_id,user_id,channel_id,local_point) SELECT ?1,?2,?3,?4 WHERE EXISTS(SELECT 1 FROM accounts WHERE server_id=?1 AND user_id=?2 AND hidden=0) ON CONFLICT(server_id,user_id,channel_id) DO UPDATE SET local_point=excluded.local_point",params![account.server().as_bytes(), account.user().as_bytes(), channel.as_bytes(), bytes])?;
    if changed != 1 {
        return Err(ReadWriteError::Session(ClientError::Closed));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
