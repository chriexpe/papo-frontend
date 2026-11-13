//! FULL expression mutations and atomic confirmed-send bookkeeping; provider transport lives separately.

use super::{ExpressionError, ExpressionSnapshot};
use crate::{
    AccountId, OperationKind, Session, outgoing::OutgoingError, registry::visible_account,
};
use rusqlite::{Transaction, params};

pub(super) async fn snapshot(session: Session) -> Result<ExpressionSnapshot, ExpressionError> {
    let registry = session.client.inner.registry.clone();
    registry
        .readers
        .query(registry.path.clone(), move |tx| {
            let _lease = registry.lease;
            if session.is_closed() {
                return Ok(Err(crate::ClientError::Closed.into()));
            }
            Ok(load(tx, session.account()))
        })
        .await?
}

fn load(tx: &Transaction<'_>, account: AccountId) -> Result<ExpressionSnapshot, ExpressionError> {
    visible_account(tx, account)?;
    let mut snapshot = ExpressionSnapshot::default();
    let mut favorites = tx.prepare(
        "SELECT kind,payload
                     FROM expression_favorites
                     WHERE server_id=?1
                     AND user_id=?2
                     ORDER BY used_order DESC
                     LIMIT 256",
    )?;
    let entries = favorites.query_map(
        params![account.server().as_bytes(), account.user().as_bytes()],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?)),
    )?;
    for entry in entries {
        let (kind, bytes) = entry?;
        match kind.as_str() {
            "gif" => snapshot
                .favorite_gifs
                .push(serde_json::from_slice(&bytes).map_err(|_| ExpressionError::InvalidData)?),
            "sticker" => snapshot
                .favorite_stickers
                .push(serde_json::from_slice(&bytes).map_err(|_| ExpressionError::InvalidData)?),
            _ => return Err(ExpressionError::InvalidData),
        }
    }
    let mut recents = tx.prepare(
        "SELECT payload
                     FROM expression_recent
                     WHERE server_id=?1
                     AND user_id=?2
                     ORDER BY used_order DESC
                     LIMIT 64",
    )?;
    for entry in recents.query_map(
        params![account.server().as_bytes(), account.user().as_bytes()],
        |row| row.get::<_, Vec<u8>>(0),
    )? {
        snapshot
            .recent_gifs
            .push(serde_json::from_slice(&entry?).map_err(|_| ExpressionError::InvalidData)?);
    }
    let mut usage = tx.prepare(
        "SELECT id,count
                     FROM expression_usage
                     WHERE server_id=?1
                     AND user_id=?2
                     ORDER BY count DESC,used_order DESC
                     LIMIT 256",
    )?;
    snapshot.reaction_usage = usage
        .query_map(
            params![account.server().as_bytes(), account.user().as_bytes()],
            |row| {
                let count = row.get::<_, i64>(1)?;
                Ok((
                    row.get(0)?,
                    u64::try_from(count).map_err(|_| rusqlite::Error::InvalidQuery)?,
                ))
            },
        )?
        .collect::<Result<_, _>>()?;
    Ok(snapshot)
}

pub(super) async fn favorite(
    session: Session,
    kind: &'static str,
    id: String,
    payload: Vec<u8>,
    favorite: bool,
) -> Result<(), ExpressionError> {
    let registry = session.client.inner.registry.clone();
    registry
        .writer
        .write(move |tx| {
            if session.is_closed() {
                return Err(crate::ClientError::Closed.into());
            }
            let account = session.account();
            visible_account(tx, account)?;
            if favorite {
                let order = next_order(tx, "expression_favorites")?;
                tx.execute(
                    "INSERT INTO expression_favorites(server_id,user_id,kind,id,payload,used_order)
                     VALUES(?1,?2,?3,?4,?5,?6)
                     ON CONFLICT(server_id,user_id,kind,id)
                     DO UPDATE SET payload=excluded.payload,used_order=excluded.used_order",
                    params![
                        account.server().as_bytes(),
                        account.user().as_bytes(),
                        kind,
                        id,
                        payload,
                        order
                    ],
                )?;
                tx.execute(
                    "DELETE
                     FROM expression_favorites
                     WHERE server_id=?1
                     AND user_id=?2
                     AND kind=?3
                     AND id NOT IN (SELECT id
                     FROM expression_favorites
                     WHERE server_id=?1
                     AND user_id=?2
                     AND kind=?3
                     ORDER BY used_order DESC
                     LIMIT 128)",
                    params![account.server().as_bytes(), account.user().as_bytes(), kind],
                )?;
            } else {
                tx.execute(
                    "DELETE
                     FROM expression_favorites
                     WHERE server_id=?1
                     AND user_id=?2
                     AND kind=?3
                     AND id=?4",
                    params![
                        account.server().as_bytes(),
                        account.user().as_bytes(),
                        kind,
                        id
                    ],
                )?;
            }
            Ok(())
        })
        .await
}

fn next_order(tx: &Transaction<'_>, table: &str) -> Result<i64, rusqlite::Error> {
    tx.query_row(
        &format!("SELECT coalesce(max(used_order),0)+1 FROM {table}"),
        [],
        |row| row.get(0),
    )
}

/// Runs in the same FULL transaction that transitions the operation to Sent, before compaction.
pub(crate) fn confirmed(
    tx: &Transaction<'_>,
    account: AccountId,
    kind: &OperationKind,
) -> Result<(), OutgoingError> {
    match kind {
        OperationKind::Message(message) => {
            if let Some(gif) = &message.gif {
                let order = next_order(tx, "expression_recent")?;
                let payload = serde_json::to_vec(gif).map_err(|_| OutgoingError::InvalidData)?;
                tx.execute(
                    "INSERT INTO expression_recent(server_id,user_id,id,payload,used_order)
                     VALUES(?1,?2,?3,?4,?5)
                     ON CONFLICT(server_id,user_id,id)
                     DO UPDATE SET payload=excluded.payload,used_order=excluded.used_order",
                    params![
                        account.server().as_bytes(),
                        account.user().as_bytes(),
                        gif.id,
                        payload,
                        order
                    ],
                )?;
                prune(tx, account, "expression_recent", 64)?;
            }
        }
        OperationKind::Reaction {
            emoji_id,
            unicode,
            add: true,
            ..
        } => {
            let id =
                emoji_id.map_or_else(|| unicode.clone().unwrap_or_default(), |id| id.to_string());
            let order = next_order(tx, "expression_usage")?;
            tx.execute(
                "INSERT INTO expression_usage(server_id,user_id,id,count,used_order)
                 VALUES(?1,?2,?3,1,?4)
                 ON CONFLICT(server_id,user_id,id)
                 DO UPDATE SET count=min(count+1,9223372036854775807),
                 used_order=excluded.used_order",
                params![
                    account.server().as_bytes(),
                    account.user().as_bytes(),
                    id,
                    order
                ],
            )?;
            prune(tx, account, "expression_usage", 256)?;
        }
        _ => {}
    }
    Ok(())
}

fn prune(
    tx: &Transaction<'_>,
    account: AccountId,
    table: &str,
    limit: i64,
) -> Result<(), rusqlite::Error> {
    tx.execute(
        &format!(
            "DELETE
                     FROM {table}
                     WHERE server_id=?1
                     AND user_id=?2
                     AND id NOT IN (SELECT id
                     FROM {table}
                     WHERE server_id=?1
                     AND user_id=?2
                     ORDER BY used_order DESC
                     LIMIT ?3)"
        ),
        params![
            account.server().as_bytes(),
            account.user().as_bytes(),
            limit
        ],
    )?;
    Ok(())
}
