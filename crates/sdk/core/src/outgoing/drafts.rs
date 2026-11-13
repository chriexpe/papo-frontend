//! Durable channel draft persistence; outgoing operations and attachment I/O live elsewhere.

use super::persistence::{data_root, decode, encode, visible};
use super::{AccountId, Arc, Draft, DraftBody, OutgoingError, Uuid, files};
use crate::{Registry, Session};
use rusqlite::{OptionalExtension, Transaction, params};
use std::path::Path;

const DRAFT_SELECT_SQL: &str = "
    SELECT revision, body, updated_at_us, files
    FROM drafts
    WHERE server_id = ?1 AND user_id = ?2 AND channel_id = ?3";

const DRAFT_REVISION_SQL: &str = "
    SELECT revision
    FROM drafts
    WHERE server_id = ?1 AND user_id = ?2 AND channel_id = ?3";

const DRAFT_UPSERT_SQL: &str = "
    INSERT INTO drafts (
        server_id, user_id, channel_id, text, mentions, updated_at_us, body, revision, files
    )
    VALUES (?1, ?2, ?3, '', x'', ?4, ?5, ?6, ?7)
    ON CONFLICT (server_id, user_id, channel_id) DO UPDATE SET
        body = excluded.body,
        files = excluded.files,
        revision = excluded.revision,
        updated_at_us = excluded.updated_at_us";

const DRAFT_CLEAR_CAPTURED_SQL: &str = "
    UPDATE drafts
    SET body = NULL, files = NULL, text = '', mentions = x'', reply_to = NULL,
        revision = revision + 1
    WHERE server_id = ?1
      AND user_id = ?2
      AND (channel_id, revision) IN (
          SELECT channel_id, draft_revision
          FROM operations
          WHERE local_id = ?3
      )";

impl Registry {
    pub(super) async fn draft_load(
        &self,
        session: Session,
        channel: Uuid,
    ) -> Result<Option<Draft>, OutgoingError> {
        let root = data_root(self)?;
        let lease = Arc::clone(&self.lease);
        self.readers
            .query(self.path.clone(), move |tx| {
                let _lease = lease;
                Ok(load_in_tx(tx, &session, channel, &root))
            })
            .await
            .map_err(OutgoingError::from)?
    }

    pub(super) async fn draft_save(
        &self,
        session: Session,
        channel: Uuid,
        mut body: DraftBody,
        updated_at_us: i64,
        expected: Option<u64>,
    ) -> Result<u64, OutgoingError> {
        body.message.validate()?;
        let bytes = files::detach(&mut body.message.attachments);
        let (references, _) = self.stage_files(&session, bytes, channel).await?;
        self.writer
            .write(move |tx| {
                visible(tx, &session)?;
                let account = session.account();
                let current: i64 = tx
                    .query_row(
                        DRAFT_REVISION_SQL,
                        params![
                            account.server().as_bytes(),
                            account.user().as_bytes(),
                            channel.as_bytes()
                        ],
                        |row| row.get(0),
                    )
                    .optional()?
                    .unwrap_or(0);
                if expected.is_some_and(|want| i64::try_from(want).ok() != Some(current)) {
                    return Err(OutgoingError::StaleDraft);
                }
                let next_revision = current.checked_add(1).ok_or(OutgoingError::InvalidData)?;
                tx.execute(
                    DRAFT_UPSERT_SQL,
                    params![
                        account.server().as_bytes(),
                        account.user().as_bytes(),
                        channel.as_bytes(),
                        updated_at_us,
                        encode(&body)?,
                        next_revision,
                        encode(&references)?
                    ],
                )?;
                u64::try_from(next_revision).map_err(|_| OutgoingError::InvalidData)
            })
            .await
    }
}

pub(super) fn verify_revision(
    tx: &Transaction<'_>,
    account: AccountId,
    channel: Uuid,
    expected: u64,
) -> Result<(), OutgoingError> {
    let current: i64 = tx.query_row(
        DRAFT_REVISION_SQL,
        params![
            account.server().as_bytes(),
            account.user().as_bytes(),
            channel.as_bytes()
        ],
        |row| row.get(0),
    )?;
    if u64::try_from(current).ok() != Some(expected) {
        return Err(OutgoingError::StaleDraft);
    }
    Ok(())
}

pub(super) fn clear_captured(
    tx: &Transaction<'_>,
    account: AccountId,
    local: Uuid,
) -> Result<(), OutgoingError> {
    // Clear only the captured revision; a newer draft is unrelated device-owned work.
    tx.execute(
        DRAFT_CLEAR_CAPTURED_SQL,
        params![
            account.server().as_bytes(),
            account.user().as_bytes(),
            local.as_bytes()
        ],
    )?;
    Ok(())
}

fn load_in_tx(
    tx: &Transaction<'_>,
    session: &Session,
    channel: Uuid,
    root: &Path,
) -> Result<Option<Draft>, OutgoingError> {
    visible(tx, session)?;
    load_row(tx, session.account(), channel, root)
}

fn load_row(
    tx: &Transaction<'_>,
    account: AccountId,
    channel: Uuid,
    root: &Path,
) -> Result<Option<Draft>, OutgoingError> {
    type DraftRow = (i64, Option<Vec<u8>>, i64, Option<Vec<u8>>);
    let row: Option<DraftRow> = tx
        .query_row(
            DRAFT_SELECT_SQL,
            params![
                account.server().as_bytes(),
                account.user().as_bytes(),
                channel.as_bytes()
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    let Some((revision, body, updated_at_us, references)) = row else {
        return Ok(None);
    };
    let mut body: DraftBody =
        body.map_or_else(|| Ok(DraftBody::default()), |bytes| decode(&bytes))?;
    let references = references.map_or_else(|| Ok(Vec::new()), |bytes| decode(&bytes))?;
    files::load(root, account, &mut body.message.attachments, &references)?;
    Ok(Some(Draft {
        revision: u64::try_from(revision).map_err(|_| OutgoingError::InvalidData)?,
        body,
        updated_at_us,
    }))
}
