//! The kept catalog in the account's durable state: read whole, replaced whole in one
//! transaction. Listing and publication live in `refresh`.

use super::{CustomEmoji, EmojiCatalogError};
use crate::{AccountId, Session, Uuid, registry::visible_account};
use rusqlite::{Transaction, params};

/// The kept catalog, oldest first.
pub(super) async fn load(session: Session) -> Result<Vec<CustomEmoji>, EmojiCatalogError> {
    let registry = session.client.inner.registry.clone();
    registry
        .readers
        .query(registry.path.clone(), move |tx| {
            let _lease = registry.lease;
            if session.is_closed() {
                return Ok(Err(crate::ClientError::Closed.into()));
            }
            Ok(read(tx, session.account()))
        })
        .await?
}

fn read(tx: &Transaction<'_>, account: AccountId) -> Result<Vec<CustomEmoji>, EmojiCatalogError> {
    visible_account(tx, account)?;
    let mut statement = tx.prepare(
        "SELECT id,name,format,created_at
         FROM emoji_catalog
         WHERE server_id=?1
         AND user_id=?2
         ORDER BY position",
    )?;
    let rows = statement.query_map(
        params![account.server().as_bytes(), account.user().as_bytes()],
        |row| {
            let id: Vec<u8> = row.get(0)?;
            Ok((id, row.get(1)?, row.get(2)?, row.get(3)?))
        },
    )?;
    let mut emojis = Vec::new();
    for row in rows {
        let (id, name, format, created_at) = row?;
        let id = Uuid::from_slice(&id).map_err(|_| crate::StorageError::InvalidState)?;
        emojis.push(CustomEmoji {
            id,
            name,
            format,
            created_at,
        });
    }
    Ok(emojis)
}

/// Replaces the kept catalog with `emojis` in one transaction, unless it already holds exactly
/// them. Returns whether anything changed.
pub(super) async fn replace(
    session: Session,
    emojis: Vec<CustomEmoji>,
) -> Result<bool, EmojiCatalogError> {
    let registry = session.client.inner.registry.clone();
    registry
        .writer
        .write(move |tx| {
            if session.is_closed() {
                return Err(crate::ClientError::Closed.into());
            }
            let account = session.account();
            visible_account(tx, account)?;
            // Kept rows that can't be read are replaced like any other change.
            if read(tx, account).is_ok_and(|kept| kept == emojis) {
                return Ok(false);
            }
            tx.execute(
                "DELETE FROM emoji_catalog WHERE server_id=?1 AND user_id=?2",
                params![account.server().as_bytes(), account.user().as_bytes()],
            )?;
            let mut insert = tx.prepare(
                "INSERT INTO emoji_catalog(server_id,user_id,id,name,format,created_at,position)
                 VALUES(?1,?2,?3,?4,?5,?6,?7)",
            )?;
            for (position, emoji) in emojis.iter().enumerate() {
                insert.execute(params![
                    account.server().as_bytes(),
                    account.user().as_bytes(),
                    emoji.id.as_bytes(),
                    emoji.name,
                    emoji.format,
                    emoji.created_at,
                    i64::try_from(position).unwrap_or(i64::MAX),
                ])?;
            }
            Ok(true)
        })
        .await
}
