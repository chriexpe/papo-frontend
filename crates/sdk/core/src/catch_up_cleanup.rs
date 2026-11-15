//! Durable synchronous=FULL SQLite deletion of reading state for revoked or removed channels.
//! Catch-up interval writes and hydration live in `catch_up_store`; saved position writes live
//! in `catch_up_position_store`.

use std::collections::BTreeSet;

use papo_api::Uuid;
use rusqlite::{Transaction, params};

use crate::{
    AccountId, Registry, StorageError,
    registry::{account_is_visible, visible_account},
};

fn delete_channel_rows_tx(
    tx: &Transaction<'_>,
    account: AccountId,
    channel: Uuid,
) -> Result<(), StorageError> {
    let server = account.server();
    let user = account.user();
    let p = params![server.as_bytes(), user.as_bytes(), channel.as_bytes()];
    tx.execute(
        "DELETE FROM saved_reading_positions WHERE server_id=?1 AND user_id=?2 AND channel_id=?3",
        p,
    )?;
    tx.execute(
        "DELETE FROM catch_up_intervals WHERE server_id=?1 AND user_id=?2 AND channel_id=?3",
        p,
    )?;
    Ok(())
}

impl Registry {
    pub(crate) async fn delete_channel_reading_state(
        &self,
        account: AccountId,
        channels: Vec<Uuid>,
    ) -> Result<(), StorageError> {
        if channels.is_empty() {
            return Ok(());
        }
        self.writer
            .write(move |tx| {
                if !account_is_visible(tx, account)? {
                    return Ok(());
                }
                for channel in channels {
                    delete_channel_rows_tx(tx, account, channel)?;
                }
                Ok(())
            })
            .await
    }

    pub(crate) async fn retain_channel_reading_state(
        &self,
        account: AccountId,
        visible: BTreeSet<Uuid>,
    ) -> Result<Vec<Uuid>, StorageError> {
        self.writer
            .write(move |tx| {
                visible_account(tx, account)?;
                let mut removed = Vec::new();
                {
                    let mut stmt = tx.prepare(
                        "SELECT channel_id FROM saved_reading_positions WHERE server_id=?1 AND user_id=?2 \
                         UNION \
                         SELECT channel_id FROM catch_up_intervals WHERE server_id=?1 AND user_id=?2",
                    )?;
                    let rows = stmt.query_map(
                        params![account.server().as_bytes(), account.user().as_bytes()],
                        |row| row.get::<_, Vec<u8>>(0),
                    )?;
                    for row in rows {
                        let channel =
                            Uuid::from_slice(&row?).map_err(|_| StorageError::InvalidState)?;
                        if !visible.contains(&channel) {
                            removed.push(channel);
                        }
                    }
                }
                for &channel in &removed {
                    delete_channel_rows_tx(tx, account, channel)?;
                }
                Ok(removed)
            })
            .await
    }
}
