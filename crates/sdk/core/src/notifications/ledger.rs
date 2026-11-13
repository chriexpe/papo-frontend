//! Existing FULL state.sqlite notification claims and bounded retention; platform handoff happens after commit.

use super::{NotificationError, NotificationSettings, inbox::InboxCursor};
use crate::{Generation, Session, registry::visible_account};
use rusqlite::{OptionalExtension, Transaction, params};

type StoredState = (NotificationSettings, Option<InboxCursor>);
type EncodedState = (Vec<u8>, Option<Vec<u8>>);

pub(crate) async fn settings(session: &Session) -> Result<NotificationSettings, NotificationError> {
    let (settings, _) = load(session).await?;
    Ok(settings)
}
pub(crate) async fn load(session: &Session) -> Result<StoredState, NotificationError> {
    let registry = session.client.inner.registry.clone();
    let session = session.clone();
    let generation = session.generation();
    registry
        .readers
        .query(registry.path.clone(), move |tx| {
            let _lease = registry.lease;
            Ok(load_transaction(tx, &session, generation))
        })
        .await?
}
fn load_transaction(
    tx: &Transaction<'_>,
    session: &Session,
    generation: Generation,
) -> Result<StoredState, NotificationError> {
    session.inner.epochs.validate(generation)?;
    let account = session.account();
    visible_account(tx, account)?;
    let row: Option<EncodedState> = tx
        .query_row(
            "SELECT settings,cursor FROM notification_state WHERE server_id=?1 AND user_id=?2",
            params![account.server().as_bytes(), account.user().as_bytes()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    let Some((settings, cursor)) = row else {
        return Ok((NotificationSettings::default(), None));
    };
    let settings = serde_json::from_slice(&settings).map_err(|_| NotificationError::InvalidData)?;
    let cursor = cursor
        .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| NotificationError::InvalidData))
        .transpose()?;
    Ok((settings, cursor))
}
pub(crate) async fn save_settings(
    session: &Session,
    settings: NotificationSettings,
) -> Result<(), NotificationError> {
    save(session, session.generation(), Some(settings), None).await
}
pub(crate) async fn save_cursor(
    session: &Session,
    generation: Generation,
    cursor: InboxCursor,
) -> Result<(), NotificationError> {
    save(session, generation, None, Some(cursor)).await
}
async fn save(
    session: &Session,
    generation: Generation,
    settings: Option<NotificationSettings>,
    cursor: Option<InboxCursor>,
) -> Result<(), NotificationError> {
    let registry = session.client.inner.registry.clone();
    let session = session.clone();
    let settings = settings
        .map(|settings| serde_json::to_vec(&settings))
        .transpose()
        .map_err(|_| NotificationError::InvalidData)?;
    let cursor = cursor
        .map(|cursor| serde_json::to_vec(&cursor))
        .transpose()
        .map_err(|_| NotificationError::InvalidData)?;
    registry
        .writer
        .write(move |tx| {
            session.inner.epochs.apply(generation, || {
                let account = session.account();
                visible_account(tx, account)?;
                tx.execute(
                    "INSERT INTO notification_state(server_id,user_id,settings,cursor)
                 VALUES(?1,?2,coalesce(?3,cast('{}' AS BLOB)),?4)
                 ON CONFLICT(server_id,user_id) DO UPDATE
                 SET settings=coalesce(?3,settings),cursor=coalesce(?4,cursor)",
                    params![
                        account.server().as_bytes(),
                        account.user().as_bytes(),
                        settings,
                        cursor
                    ],
                )?;
                Ok::<_, NotificationError>(())
            })?
        })
        .await
}
#[cfg(test)]
pub(crate) async fn claim(
    session: &Session,
    generation: Generation,
    message: crate::Uuid,
    delivered_at_us: i64,
) -> Result<bool, NotificationError> {
    claim_candidate(
        session,
        generation,
        message,
        delivered_at_us,
        delivered_at_us,
    )
    .await
}

pub(crate) async fn claim_candidate(
    session: &Session,
    generation: Generation,
    message: crate::Uuid,
    observed_at_us: i64,
    delivered_at_us: i64,
) -> Result<bool, NotificationError> {
    let registry = session.client.inner.registry.clone();
    let session = session.clone();
    registry
        .writer
        .write(move |tx| {
            session.inner.epochs.apply(generation, || {
                claim_transaction(
                    tx,
                    session.account(),
                    message,
                    observed_at_us,
                    delivered_at_us,
                )
            })?
        })
        .await
}

fn claim_transaction(
    tx: &Transaction<'_>,
    account: crate::AccountId,
    message: crate::Uuid,
    observed_at_us: i64,
    delivered_at_us: i64,
) -> Result<bool, NotificationError> {
    visible_account(tx, account)?;
    // Both admission and collection use canonical message time. A retained monotonic
    // floor prevents a later local clock rollback from reviving a collected identity.
    let oldest_us = delivered_at_us.saturating_sub(30 * 24 * 60 * 60 * 1_000_000);
    tx.execute(
        "INSERT INTO notification_state(server_id,user_id,not_before_us) VALUES(?1,?2,?3)
         ON CONFLICT(server_id,user_id) DO UPDATE SET not_before_us=max(not_before_us,?3)",
        params![
            account.server().as_bytes(),
            account.user().as_bytes(),
            oldest_us
        ],
    )?;
    let floor_us: i64 = tx.query_row(
        "SELECT not_before_us FROM notification_state WHERE server_id=?1 AND user_id=?2",
        params![account.server().as_bytes(), account.user().as_bytes()],
        |row| row.get(0),
    )?;
    if observed_at_us < floor_us {
        return Ok(false);
    }
    tx.execute(
        "DELETE FROM notification_ledger WHERE rowid IN (
         SELECT rowid FROM notification_ledger WHERE server_id=?1 AND user_id=?2
         AND message_created_at_us<?3 ORDER BY message_created_at_us LIMIT 100)",
        params![
            account.server().as_bytes(),
            account.user().as_bytes(),
            floor_us
        ],
    )?;
    let changed = tx.execute(
        "INSERT OR IGNORE INTO notification_ledger(
         server_id,user_id,message_id,delivered_at_us,message_created_at_us)
         VALUES(?1,?2,?3,?4,?5)",
        params![
            account.server().as_bytes(),
            account.user().as_bytes(),
            message.as_bytes(),
            delivered_at_us,
            observed_at_us
        ],
    )?;
    Ok(changed == 1)
}
