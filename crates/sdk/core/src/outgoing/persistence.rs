//! FULL durable operation rows and account-fenced transitions; drafts and request execution live separately.

use super::{
    AccountId, Arc, ClientError, Message, Operation, OperationFailure, OperationKind,
    OperationState, OutgoingError, OutgoingSnapshot, Uuid, drafts, files,
};
use crate::{Registry, Session, registry::visible_account};
use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const LIST_OPERATIONS_SQL: &str = "
    SELECT
        local_id, channel_id, payload, state, attempts, depends_on, last_error, result,
        created_at_us, files
    FROM operations
    WHERE server_id = ?1 AND user_id = ?2 AND discarded = 0
    ORDER BY rowid";

const MARK_PROJECTED_SQL: &str = "
    UPDATE operations
    SET result = CAST(json_set(result, '$.projected', json('true')) AS BLOB)
    WHERE local_id = ?1 AND server_id = ?2 AND user_id = ?3 AND state = 'Sent'";

const TRANSITION_STATE_SQL: &str = "
    UPDATE operations
    SET state = ?1, attempts = attempts + ?2, result = ?3, last_error = ?4
    WHERE local_id = ?5 AND server_id = ?6 AND user_id = ?7 AND state = ?8 AND discarded = 0";

const SENT_PAYLOAD_SQL: &str = "
    UPDATE operations
    SET payload = ?1, depends_on = NULL, confirmed_order = ?3, files = NULL, attachment_bytes = 0
    WHERE local_id = ?2";

const COMPACT_SENT_SQL: &str = "
    DELETE FROM operations
    WHERE server_id = ?1
      AND user_id = ?2
      AND ((state = 'Sent' AND json_extract(result, '$.projected') = 1) OR discarded = 1)
      AND NOT EXISTS (
          SELECT 1
          FROM operations child
          WHERE child.depends_on = operations.local_id
      )
      AND rowid NOT IN (
          SELECT rowid
          FROM operations
          WHERE server_id = ?1 AND user_id = ?2 AND state = 'Sent' AND discarded = 0
          ORDER BY confirmed_order DESC
          LIMIT 256
      )";

const HAS_ACTIVE_DEPENDENTS_SQL: &str = "
    SELECT EXISTS(
        SELECT 1
        FROM operations
        WHERE depends_on = ?1 AND discarded = 0 AND state != 'Sent'
    )";

const IS_SENT_SQL: &str = "
    SELECT EXISTS(
        SELECT 1
        FROM operations
        WHERE local_id = ?1 AND server_id = ?2 AND user_id = ?3 AND state = 'Sent'
    )";

const MARK_DISCARDED_SQL: &str = "
    UPDATE operations
    SET discarded = 1
    WHERE local_id = ?1
      AND server_id = ?2
      AND user_id = ?3
      AND state != 'Sending'
      AND discarded = 0";

const FAIL_QUEUED_DEPENDENTS_SQL: &str = "
    UPDATE operations
    SET state = 'Failed', last_error = ?1
    WHERE depends_on = ?2 AND server_id = ?3 AND user_id = ?4 AND state = 'Queued'";

const CLEAR_DISCARDED_PAYLOAD_SQL: &str = "
    UPDATE operations
    SET payload = x'', result = NULL, files = NULL, attachment_bytes = 0
    WHERE local_id = ?1 AND discarded = 1";

const COMPACT_DISCARDED_SQL: &str = "
    DELETE FROM operations
    WHERE server_id = ?1
      AND user_id = ?2
      AND discarded = 1
      AND NOT EXISTS (
          SELECT 1
          FROM operations child
          WHERE child.depends_on = operations.local_id
      )
      AND rowid NOT IN (
          SELECT rowid
          FROM operations
          WHERE server_id = ?1 AND user_id = ?2 AND discarded = 1
          ORDER BY rowid DESC
          LIMIT 256
      )";

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Outcome {
    pub message_id: Option<Uuid>,
    pub message: Option<Message>,
    #[serde(default)]
    pub projected: bool,
}

pub(super) fn data_root(registry: &Registry) -> Result<PathBuf, OutgoingError> {
    registry
        .path
        .parent()
        .ok_or(OutgoingError::InvalidData)
        .map(Path::to_path_buf)
}

pub(super) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, OutgoingError> {
    serde_json::to_vec(value).map_err(|_| OutgoingError::InvalidData)
}

pub(super) fn decode<T: DeserializeOwned>(value: &[u8]) -> Result<T, OutgoingError> {
    serde_json::from_slice(value).map_err(|_| OutgoingError::InvalidData)
}

pub(super) fn visible(tx: &Transaction<'_>, session: &Session) -> Result<(), OutgoingError> {
    if session.is_closed() {
        return Err(ClientError::Closed.into());
    }
    visible_account(tx, session.account())?;
    Ok(())
}

fn uuid_column(bytes: &[u8]) -> Result<Uuid, OutgoingError> {
    Uuid::from_slice(bytes).map_err(|_| OutgoingError::InvalidData)
}

fn opt_uuid_column(bytes: Option<Vec<u8>>) -> Result<Option<Uuid>, OutgoingError> {
    bytes.map(|value| uuid_column(&value)).transpose()
}

pub(super) fn opt_decode<T: DeserializeOwned>(
    bytes: Option<Vec<u8>>,
) -> Result<Option<T>, OutgoingError> {
    bytes.map(|value| decode(&value)).transpose()
}

fn read_operation(
    row: &rusqlite::Row<'_>,
    root: &Path,
    account: AccountId,
    previous: &BTreeMap<Uuid, Arc<Operation>>,
) -> Result<Operation, OutgoingError> {
    let outcome: Outcome = opt_decode(row.get(7)?)?.unwrap_or_default();
    let state = OperationState::from_db(&row.get::<_, String>(3)?)?;
    let id = uuid_column(&row.get::<_, Vec<u8>>(0)?)?;
    let payload: Vec<u8> = row.get(2)?;
    let mut kind: OperationKind = decode(&payload)?;
    let retained = previous
        .get(&id)
        .filter(|op| op.state != OperationState::Sent && state != OperationState::Sent);
    if retained.is_none()
        && state != OperationState::Sent
        && let OperationKind::Message(message) = &mut kind
    {
        let references: Vec<files::Reference> = opt_decode(row.get(9)?)?.unwrap_or_default();
        files::load(root, account, &mut message.attachments, &references)?;
    }
    Ok(Operation {
        id,
        channel_id: uuid_column(&row.get::<_, Vec<u8>>(1)?)?,
        kind: retained.map_or_else(|| Arc::new(kind), |op| Arc::clone(&op.kind)),
        state,
        attempts: row.get(4)?,
        depends_on: opt_uuid_column(row.get(5)?)?,
        failure: opt_decode(row.get(6)?)?,
        message_id: outcome.message_id,
        history_projected: outcome.projected,
        message: outcome.message.map(Arc::new),
        created_at_us: row.get(8)?,
    })
}

fn list_operations(
    tx: &Transaction<'_>,
    session: &Session,
    root: &Path,
    previous: &BTreeMap<Uuid, Arc<Operation>>,
) -> Result<Vec<Arc<Operation>>, OutgoingError> {
    visible(tx, session)?;
    let account = session.account();
    let mut statement = tx.prepare(LIST_OPERATIONS_SQL)?;
    let mut rows = statement.query(params![
        account.server().as_bytes(),
        account.user().as_bytes()
    ])?;
    let mut values = Vec::new();
    while let Some(row) = rows.next()? {
        values.push(Arc::new(read_operation(row, root, account, previous)?));
    }
    Ok(values)
}

fn finalize_sent(
    tx: &Transaction<'_>,
    account: AccountId,
    local: Uuid,
) -> Result<(), OutgoingError> {
    let bytes: Vec<u8> = tx.query_row(
        "SELECT payload FROM operations WHERE local_id = ?1",
        [local.as_bytes()],
        |row| row.get(0),
    )?;
    let mut kind: OperationKind = decode(&bytes)?;
    crate::expressions::confirmed(tx, account, &kind)?;
    if let OperationKind::Message(message) = &mut kind {
        for file in &mut message.attachments {
            file.bytes.clear();
        }
    }
    let order: i64 = tx.query_row(
        "SELECT coalesce(max(confirmed_order), 0) FROM operations",
        [],
        |row| row.get(0),
    )?;
    let next_order = order.checked_add(1).ok_or(OutgoingError::InvalidData)?;
    tx.execute(
        SENT_PAYLOAD_SQL,
        params![encode(&kind)?, local.as_bytes(), next_order],
    )?;
    // Only confirmed receipts are compacted; unresolved intent and required mappings survive.
    tx.execute(
        COMPACT_SENT_SQL,
        params![account.server().as_bytes(), account.user().as_bytes()],
    )?;
    drafts::clear_captured(tx, account, local)
}

fn discard_operation(
    tx: &Transaction<'_>,
    account: AccountId,
    local: Uuid,
) -> Result<(), OutgoingError> {
    let has_dependents: bool =
        tx.query_row(HAS_ACTIVE_DEPENDENTS_SQL, [local.as_bytes()], |row| {
            row.get(0)
        })?;
    let is_sent: bool = tx.query_row(
        IS_SENT_SQL,
        params![
            local.as_bytes(),
            account.server().as_bytes(),
            account.user().as_bytes()
        ],
        |row| row.get(0),
    )?;
    if is_sent && has_dependents {
        return Err(OutgoingError::InvalidState);
    }
    let discarded_rows = tx.execute(
        MARK_DISCARDED_SQL,
        params![
            local.as_bytes(),
            account.server().as_bytes(),
            account.user().as_bytes()
        ],
    )?;
    if discarded_rows != 1 {
        return Err(OutgoingError::InvalidState);
    }
    tx.execute(
        FAIL_QUEUED_DEPENDENTS_SQL,
        params![
            encode(&OperationFailure::Dependency)?,
            local.as_bytes(),
            account.server().as_bytes(),
            account.user().as_bytes()
        ],
    )?;
    tx.execute(CLEAR_DISCARDED_PAYLOAD_SQL, [local.as_bytes()])?;
    tx.execute(
        COMPACT_DISCARDED_SQL,
        params![account.server().as_bytes(), account.user().as_bytes()],
    )?;
    Ok(())
}

impl Registry {
    pub(super) async fn outgoing_projected(
        &self,
        session: Session,
        ids: Vec<Uuid>,
    ) -> Result<(), OutgoingError> {
        self.writer
            .write(move |tx| {
                visible(tx, &session)?;
                let account = session.account();
                for id in ids {
                    tx.execute(
                        MARK_PROJECTED_SQL,
                        params![
                            id.as_bytes(),
                            account.server().as_bytes(),
                            account.user().as_bytes()
                        ],
                    )?;
                }
                Ok(())
            })
            .await
    }

    pub(super) async fn outgoing_list(
        &self,
        session: Session,
        previous: Arc<OutgoingSnapshot>,
    ) -> Result<Vec<Arc<Operation>>, OutgoingError> {
        let root = data_root(self)?;
        let lease = Arc::clone(&self.lease);
        let previous: BTreeMap<_, _> = previous
            .operations
            .iter()
            .map(|op| (op.id, Arc::clone(op)))
            .collect();
        self.readers
            .query(self.path.clone(), move |tx| {
                let _lease = lease;
                Ok(list_operations(tx, &session, &root, &previous))
            })
            .await
            .map_err(OutgoingError::from)?
    }

    pub(super) async fn outgoing_transition(
        &self,
        session: Session,
        local: Uuid,
        expected: OperationState,
        next: OperationState,
        outcome: Option<Outcome>,
        failure: Option<OperationFailure>,
    ) -> Result<(), OutgoingError> {
        let notifications = session.inner.expression_changes.clone();
        self.writer
            .write(move |tx| {
                visible(tx, &session)?;
                let account = session.account();
                let changed = tx.execute(
                    TRANSITION_STATE_SQL,
                    params![
                        next.as_str(),
                        u32::from(next == OperationState::Sending),
                        outcome.map(|value| encode(&value)).transpose()?,
                        failure.map(|value| encode(&value)).transpose()?,
                        local.as_bytes(),
                        account.server().as_bytes(),
                        account.user().as_bytes(),
                        expected.as_str()
                    ],
                )?;
                if changed != 1 {
                    return Err(OutgoingError::InvalidState);
                }
                if next == OperationState::Sent {
                    finalize_sent(tx, account, local)?;
                }
                Ok(())
            })
            .await?;
        if next == OperationState::Sent {
            notifications.send_modify(|revision| *revision = revision.saturating_add(1));
        }
        Ok(())
    }

    pub(super) async fn outgoing_discard(
        &self,
        session: Session,
        local: Uuid,
    ) -> Result<(), OutgoingError> {
        self.writer
            .write(move |tx| {
                visible(tx, &session)?;
                discard_operation(tx, session.account(), local)
            })
            .await
    }
}
