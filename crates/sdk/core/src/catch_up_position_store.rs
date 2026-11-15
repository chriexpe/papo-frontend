//! Durable synchronous=FULL SQLite persistence for saved reading position writes.
//! Coalescing and revocation fencing live in `reading::positions`; catch-up interval writes and
//! hydration live in `catch_up_store`; revoked/removed channel cleanup lives in `catch_up_cleanup`.

use std::{collections::BTreeMap, sync::Arc};

use papo_api::Uuid;
use rusqlite::{Transaction, params};

use crate::{
    AccountId, Generation, HistoryClosedReason, ReadingError, Registry, StorageError,
    catch_up::ReadingPosition, generation::EpochError, registry::account_is_visible,
    session::SessionInner, storage_worker::WorkerError,
};

pub(crate) struct PendingPositionCommit {
    pub channel: Uuid,
    pub revocation_epoch: u64,
    pub position: Option<ReadingPosition>,
}

#[derive(Clone, Debug, thiserror::Error)]
pub(crate) enum PositionBatchError {
    #[error(transparent)]
    Epoch(EpochError),
    #[error("account unavailable")]
    AccountNotFound,
    #[error(transparent)]
    Worker(WorkerError),
}

impl From<EpochError> for PositionBatchError {
    fn from(error: EpochError) -> Self {
        Self::Epoch(error)
    }
}

impl From<WorkerError> for PositionBatchError {
    fn from(error: WorkerError) -> Self {
        Self::Worker(error)
    }
}

impl From<rusqlite::Error> for PositionBatchError {
    fn from(error: rusqlite::Error) -> Self {
        Self::Worker(WorkerError::from(error))
    }
}

impl From<PositionBatchError> for ReadingError {
    fn from(error: PositionBatchError) -> Self {
        match error {
            PositionBatchError::Epoch(epoch) => Self::Session(epoch.into()),
            PositionBatchError::AccountNotFound => Self::Storage(StorageError::NotFound),
            PositionBatchError::Worker(worker) => Self::Storage(StorageError::Worker(worker)),
        }
    }
}

fn upsert_position_tx(
    tx: &Transaction<'_>,
    account: AccountId,
    channel: Uuid,
    position: Option<ReadingPosition>,
) -> rusqlite::Result<()> {
    match position {
        Some(pos) => {
            tx.execute(
                "INSERT INTO saved_reading_positions(server_id,user_id,channel_id,message_id,offset_units) \
                 VALUES(?1,?2,?3,?4,?5) \
                 ON CONFLICT(server_id,user_id,channel_id) DO UPDATE SET \
                 message_id=excluded.message_id, offset_units=excluded.offset_units",
                params![
                    account.server().as_bytes(),
                    account.user().as_bytes(),
                    channel.as_bytes(),
                    pos.message_id.as_bytes(),
                    pos.offset_units,
                ],
            )?;
        }
        None => {
            tx.execute(
                "DELETE FROM saved_reading_positions WHERE server_id=?1 AND user_id=?2 AND channel_id=?3",
                params![
                    account.server().as_bytes(),
                    account.user().as_bytes(),
                    channel.as_bytes(),
                ],
            )?;
        }
    }
    Ok(())
}

impl Registry {
    pub(crate) async fn commit_position_batch(
        &self,
        session: Arc<SessionInner>,
        generation: Generation,
        batch: Vec<PendingPositionCommit>,
    ) -> Result<BTreeMap<Uuid, HistoryClosedReason>, PositionBatchError> {
        let account = session.account;
        self.writer
            .write(
                move |tx| -> Result<BTreeMap<Uuid, HistoryClosedReason>, PositionBatchError> {
                    session.epochs.validate_epoch(generation)?;
                    if !account_is_visible(tx, account)? {
                        return Err(PositionBatchError::AccountNotFound);
                    }
                    let mut rejected = BTreeMap::new();
                    for item in batch {
                        // Skip any channel racing with a concurrent revocation without rolling back siblings.
                        if let Some(reason) = session.reading.positions.check_writable(
                            item.channel,
                            generation,
                            item.revocation_epoch,
                        ) {
                            rejected.insert(item.channel, reason);
                            continue;
                        }
                        upsert_position_tx(tx, account, item.channel, item.position)?;
                    }
                    Ok(rejected)
                },
            )
            .await
    }
}
