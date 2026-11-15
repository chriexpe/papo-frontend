//! Durable synchronous=FULL SQLite persistence for saved reading positions.
//! Coalescing and revocation fencing live in `reading::positions`; catch-up intervals live in `catch_up_store`.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use papo_api::Uuid;
use rusqlite::{Transaction, params};

use crate::{
    AccountId, Generation, HistoryClosedReason, ReadingError, Registry, StorageError,
    catch_up::ReadingPosition,
    catch_up_store::{CatchUpWriteError, visible_account},
    session::SessionInner,
};

pub(crate) fn upsert_position_tx(
    tx: &Transaction<'_>,
    account: AccountId,
    channel: Uuid,
    position: Option<ReadingPosition>,
) -> Result<(), StorageError> {
    visible_account(tx, account)?;
    match position {
        Some(pos) => {
            let changed = tx.execute(
                "INSERT INTO saved_reading_positions(server_id,user_id,channel_id,message_id,offset_units) \
                 SELECT ?1,?2,?3,?4,?5 WHERE EXISTS(SELECT 1 FROM accounts a JOIN servers s ON s.id=a.server_id WHERE a.server_id=?1 AND a.user_id=?2 AND a.hidden=0 AND s.hidden=0) \
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
            if changed == 0 {
                return Err(StorageError::NotFound);
            }
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
    pub(crate) async fn delete_saved_positions(
        &self,
        account: AccountId,
        channels: Vec<Uuid>,
    ) -> Result<(), StorageError> {
        if channels.is_empty() {
            return Ok(());
        }
        self.writer
            .write(move |tx| {
                for channel in channels {
                    tx.execute(
                        "DELETE FROM saved_reading_positions WHERE server_id=?1 AND user_id=?2 AND channel_id=?3",
                        params![
                            account.server().as_bytes(),
                            account.user().as_bytes(),
                            channel.as_bytes(),
                        ],
                    )?;
                }
                Ok(())
            })
            .await
    }

    pub(crate) async fn retain_saved_positions(
        &self,
        account: AccountId,
        visible: BTreeSet<Uuid>,
    ) -> Result<Vec<Uuid>, StorageError> {
        self.writer
            .write(move |tx| {
                visible_account(tx, account)?;
                let mut existing = Vec::new();
                {
                    let mut stmt = tx.prepare(
                        "SELECT channel_id FROM saved_reading_positions WHERE server_id=?1 AND user_id=?2",
                    )?;
                    let rows = stmt.query_map(
                        params![account.server().as_bytes(), account.user().as_bytes()],
                        |row| row.get::<_, Vec<u8>>(0),
                    )?;
                    for row in rows {
                        let channel =
                            Uuid::from_slice(&row?).map_err(|_| StorageError::InvalidState)?;
                        if !visible.contains(&channel) {
                            existing.push(channel);
                        }
                    }
                }
                for &channel in &existing {
                    tx.execute(
                        "DELETE FROM saved_reading_positions WHERE server_id=?1 AND user_id=?2 AND channel_id=?3",
                        params![
                            account.server().as_bytes(),
                            account.user().as_bytes(),
                            channel.as_bytes(),
                        ],
                    )?;
                }
                Ok(existing)
            })
            .await
    }

    pub(crate) async fn commit_position_batch(
        &self,
        session: Arc<SessionInner>,
        generation: Generation,
        batch: Vec<(Uuid, u64, Option<ReadingPosition>)>,
    ) -> Result<BTreeMap<Uuid, HistoryClosedReason>, ReadingError> {
        let account = session.account;
        self.writer
            .write(
                move |tx| -> Result<BTreeMap<Uuid, HistoryClosedReason>, CatchUpWriteError> {
                    session.epochs.validate(generation)?;
                    visible_account(tx, account)?;
                    let mut rejected = BTreeMap::new();
                    for (channel, expected_epoch, position) in batch {
                        // Skip any channel racing with a concurrent revocation without rolling back siblings.
                        if let Some(reason) = session.reading.positions.check_writable(
                            channel,
                            generation,
                            expected_epoch,
                        ) {
                            rejected.insert(channel, reason);
                            continue;
                        }
                        upsert_position_tx(tx, account, channel, position)?;
                    }
                    Ok(rejected)
                },
            )
            .await
            .map_err(ReadingError::from)
    }
}
