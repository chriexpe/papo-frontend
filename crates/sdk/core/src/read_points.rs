//! Reconcile persistence of server read marks; advancing and uploading local pending points belongs to reading.

use crate::{AccountId, Registry, StorageError};
use papo_api::{
    Uuid,
    wire::{Channel, DirectConversation},
};
use rusqlite::params;

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

fn serialize_id<S: serde::Serializer>(id: &Uuid, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&id.to_string())
}
fn deserialize_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    let value = <String as serde::Deserialize>::deserialize(deserializer)?;
    value
        .parse()
        .map_err(|_| serde::de::Error::custom("invalid read point UUID"))
}

impl Registry {
    pub(crate) async fn reconcile_read_points(
        &self,
        account: AccountId,
        channels: &[Channel],
        dms: &[DirectConversation],
    ) -> Result<(), StorageError> {
        let marks = channels
            .iter()
            .map(|channel| {
                (
                    channel.id.as_deref(),
                    channel.last_read_message.as_deref(),
                    channel.last_read_at.clone(),
                )
            })
            .chain(dms.iter().map(|dm| {
                (
                    Some(dm.id.as_str()),
                    dm.last_read_message.as_deref(),
                    dm.last_read_at.clone(),
                )
            }))
            .map(|(channel, message, recorded_at)| {
                let channel: Uuid = channel
                    .ok_or(StorageError::InvalidState)?
                    .parse()
                    .map_err(|_| StorageError::InvalidState)?;
                let mark = message
                    .map(|message| {
                        let message_id = message.parse().map_err(|_| StorageError::InvalidState)?;
                        serde_json::to_vec(&ServerReadPoint {
                            message_id,
                            recorded_at,
                        })
                        .map_err(|_| StorageError::InvalidState)
                    })
                    .transpose()?;
                Ok((channel, mark))
            })
            .collect::<Result<Vec<_>, StorageError>>()?;
        for batch in marks.chunks(500) {
            let batch = batch.to_vec();
            self.writer.write(move |tx| {
                for (channel, mark) in batch {
                    // Pending bytes are deliberately untouched: only the backend can advance a read mark.
                    tx.execute("INSERT INTO read_points(server_id,user_id,channel_id,uploaded_point) SELECT ?1,?2,?3,?4 WHERE EXISTS(SELECT 1 FROM accounts WHERE server_id=?1 AND user_id=?2 AND hidden=0) ON CONFLICT(server_id,user_id,channel_id) DO UPDATE SET uploaded_point=excluded.uploaded_point", params![account.server().as_bytes(), account.user().as_bytes(), channel.as_bytes(), mark])?;
                }
                Ok::<_,StorageError>(())
            }).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Server refresh never overwrites local pending state.
    use super::*;
    use crate::{
        ServerAddress,
        storage_worker::{StorageWorker, WorkerConfig},
    };
    use std::time::{Duration, Instant};

    #[test]
    fn server_refresh_preserves_pending_read_bytes() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.sqlite");
        let (mut worker, writer) = StorageWorker::state(
            path.clone(),
            |connection| {
                crate::state_schema::migrate(connection)
                    .map_err(|_| crate::storage_worker::WorkerError::Initialization)
            },
            WorkerConfig::default(),
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let account = AccountId::new(
            ServerAddress::parse("https://example.invalid")
                .unwrap()
                .id(),
            Uuid::from_u128(1),
        );
        runtime.block_on(async {
            writer.write(move |tx| {
                tx.execute("INSERT INTO servers VALUES(?1,'https://example.invalid',0,0,0)",[account.server().as_bytes()])?;
                tx.execute("INSERT INTO accounts(server_id,user_id,position,created_at_us) VALUES(?1,?2,0,0)",params![account.server().as_bytes(),account.user().as_bytes()])?;
                tx.execute("INSERT INTO read_points VALUES(?1,?2,?3,?4,NULL)",params![account.server().as_bytes(),account.user().as_bytes(),Uuid::from_u128(2).as_bytes(),b"pending".as_slice()])?;
                Ok::<_,StorageError>(())
            }).await.unwrap();
            let data = crate::DataRoot::new(root.path().join("lease"));
            let registry = Registry { writer, readers: std::sync::Arc::new(crate::storage_worker::ReadPool::default()), path:path.clone(), lease:data.acquire().unwrap() };
            registry.reconcile_read_points(account,&[Channel { id:Some(Uuid::from_u128(2).to_string()),last_read_message:Some(Uuid::from_u128(3).to_string()),last_read_at:Some("2026-01-01T00:00:00Z".into()),..Channel::default() }],&[]).await.unwrap();
            let row: (Vec<u8>,Vec<u8>)=registry.readers.query(path,|tx|tx.query_row("SELECT local_point,uploaded_point FROM read_points",[],|row|Ok((row.get(0)?,row.get(1)?)))).await.unwrap();
            assert_eq!(row.0,b"pending");
            assert_eq!(serde_json::from_slice::<ServerReadPoint>(&row.1).unwrap().message_id,Uuid::from_u128(3));
        });
        worker
            .shutdown(Instant::now() + Duration::from_secs(2))
            .unwrap();
        worker.join().unwrap();
    }
}
