//! Operation admission checks and atomic SQLite insertion; state transitions and queries live in persistence.

use super::persistence::{encode, opt_decode, visible};
use super::{AccountId, EnqueueSource, OperationKind, OutgoingError, Uuid, drafts, files};
use crate::{Registry, Session};
use rusqlite::{OptionalExtension, Transaction, params};

const MAX_RETAINED_OPERATIONS: i64 = 256;
pub(super) const MAX_RETAINED_BYTES: i64 = 256 * 1024 * 1024;

const ADMISSION_USAGE_SQL: &str = "
    SELECT count(*), coalesce(sum(length(payload) + attachment_bytes), 0)
    FROM operations
    WHERE server_id = ?1
      AND user_id = ?2
      AND discarded = 0
      AND (state != 'Sent' OR coalesce(json_extract(result, '$.projected'), 0) = 0)";

const DEPENDENCY_PAYLOAD_SQL: &str = "
    SELECT payload
    FROM operations
    WHERE local_id = ?1 AND server_id = ?2 AND user_id = ?3 AND channel_id = ?4 AND discarded = 0";

const INSERT_OPERATION_SQL: &str = "
    INSERT INTO operations (
        local_id, server_id, user_id, channel_id, kind, payload, depends_on, state,
        created_at_us, draft_revision, files, attachment_bytes
    )
    VALUES (?1, ?2, ?3, ?4, 'sdk-v1', ?5, ?6, 'Queued', ?7, ?8, ?9, ?10)";

pub(super) fn ensure_unique_local_id(
    tx: &Transaction<'_>,
    local: Uuid,
) -> Result<(), OutgoingError> {
    let exists: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM operations WHERE local_id = ?1)",
        [local.as_bytes()],
        |row| row.get(0),
    )?;
    if exists {
        return Err(OutgoingError::AlreadyExists);
    }
    Ok(())
}

pub(super) fn check_admission_budget(
    tx: &Transaction<'_>,
    account: AccountId,
    payload_len: usize,
    attachment_bytes: i64,
) -> Result<(), OutgoingError> {
    let (count, retained_bytes): (i64, i64) = tx.query_row(
        ADMISSION_USAGE_SQL,
        params![account.server().as_bytes(), account.user().as_bytes()],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let payload_bytes = i64::try_from(payload_len).map_err(|_| OutgoingError::Budget)?;
    let total_bytes = retained_bytes
        .saturating_add(payload_bytes)
        .saturating_add(attachment_bytes);
    if count >= MAX_RETAINED_OPERATIONS || total_bytes > MAX_RETAINED_BYTES {
        return Err(OutgoingError::Budget);
    }
    Ok(())
}

pub(super) fn verify_dependency(
    tx: &Transaction<'_>,
    account: AccountId,
    channel: Uuid,
    dependency: Uuid,
) -> Result<(), OutgoingError> {
    let data: Option<Vec<u8>> = tx
        .query_row(
            DEPENDENCY_PAYLOAD_SQL,
            params![
                dependency.as_bytes(),
                account.server().as_bytes(),
                account.user().as_bytes(),
                channel.as_bytes()
            ],
            |row| row.get(0),
        )
        .optional()?;
    let decoded = opt_decode::<OperationKind>(data)?;
    if !matches!(decoded, Some(OperationKind::Message(_))) {
        return Err(OutgoingError::NotFound);
    }
    Ok(())
}

impl Registry {
    pub(super) async fn outgoing_enqueue(
        &self,
        session: Session,
        channel: Uuid,
        local: Uuid,
        source: EnqueueSource,
        created_at_us: i64,
        expected_generation: Option<crate::Generation>,
    ) -> Result<(), OutgoingError> {
        let (mut kind, draft_revision) = match source {
            EnqueueSource::Kind(kind) => (kind, None),
            EnqueueSource::Draft { revision } => {
                let draft = self
                    .draft_load(session.clone(), channel)
                    .await?
                    .ok_or(OutgoingError::StaleDraft)?;
                if draft.revision != revision {
                    return Err(OutgoingError::StaleDraft);
                }
                (OperationKind::Message(draft.body.message), Some(revision))
            }
        };
        kind.validate()?;
        if let OperationKind::Message(message) = &kind
            && super::preparing::has_copying(message)
        {
            if draft_revision.is_some() || expected_generation.is_some() {
                return Err(OutgoingError::InvalidInput);
            }
            return self
                .outgoing_enqueue_preparing(session, channel, local, kind, created_at_us)
                .await;
        }
        // Edited copies are unreferenced until the commit below; holding their handles keeps
        // background collection from deleting them in between.
        let mut held_edits = Vec::new();
        if let OperationKind::Message(message) = &mut kind {
            self.outgoing_apply_edits(&session, message, local).await?;
            message.validate()?;
            held_edits.extend(
                message
                    .attachments
                    .iter()
                    .filter_map(|file| file.staged().cloned()),
            );
        }
        let inputs = if let OperationKind::Message(message) = &mut kind {
            files::detach_inputs(&mut message.attachments, session.account())?
        } else {
            Vec::new()
        };
        let (references, attachment_bytes) =
            self.prepare_attachments(&session, inputs, local).await?;
        let payload = encode(&kind)?;
        let dependency = kind.dependency();
        let committed = self
            .writer
            .write(move |tx| {
                let admit = || {
                    visible(tx, &session)?;
                    let account = session.account();
                    if let Some(expected) = draft_revision {
                        drafts::verify_revision(tx, account, channel, expected)?;
                    }
                    ensure_unique_local_id(tx, local)?;
                    check_admission_budget(tx, account, payload.len(), attachment_bytes)?;
                    if let Some(dependency_id) = dependency {
                        verify_dependency(tx, account, channel, dependency_id)?;
                    }
                    let db_revision = draft_revision
                        .map(i64::try_from)
                        .transpose()
                        .map_err(|_| OutgoingError::InvalidData)?;
                    tx.execute(
                        INSERT_OPERATION_SQL,
                        params![
                            local.as_bytes(),
                            account.server().as_bytes(),
                            account.user().as_bytes(),
                            channel.as_bytes(),
                            payload,
                            dependency.map(|id| id.as_bytes().to_vec()),
                            created_at_us,
                            db_revision,
                            encode(&references)?,
                            attachment_bytes
                        ],
                    )?;
                    Ok(())
                };
                if let Some(generation) = expected_generation {
                    // Waiting for the shared writer must not admit an obsolete ephemeral command.
                    session.inner.epochs.apply(generation, admit)?
                } else {
                    admit()
                }
            })
            .await;
        drop(held_edits);
        committed
    }
}
