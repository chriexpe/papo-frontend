//! FULL durable operation/draft rows and account-fenced transitions; request execution lives separately.

use super::files;
use super::{
    AccountId, Arc, ClientError, Draft, DraftBody, Message, Operation, OperationFailure,
    OperationKind, OperationState, OutgoingError, Uuid,
};
use crate::{Registry, Session, registry::visible_account};
use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Outcome {
    pub message_id: Option<Uuid>,
    pub message: Option<Message>,
    #[serde(default)]
    pub projected: bool,
}
fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, OutgoingError> {
    serde_json::to_vec(value).map_err(|_| OutgoingError::InvalidData)
}
fn decode<T: serde::de::DeserializeOwned>(value: &[u8]) -> Result<T, OutgoingError> {
    serde_json::from_slice(value).map_err(|_| OutgoingError::InvalidData)
}
pub(super) fn visible(tx: &Transaction<'_>, session: &Session) -> Result<(), OutgoingError> {
    if session.is_closed() {
        return Err(ClientError::Closed.into());
    }
    visible_account(tx, session.account())?;
    Ok(())
}
fn read_operation(
    row: &rusqlite::Row<'_>,
    root: &Path,
    account: AccountId,
    previous: &std::collections::BTreeMap<Uuid, Arc<Operation>>,
) -> Result<Operation, OutgoingError> {
    let outcome: Outcome = row
        .get::<_, Option<Vec<u8>>>(7)?
        .map_or_else(|| Ok(Outcome::default()), |v| decode(&v))?;
    let state = match row.get::<_, String>(3)?.as_str() {
        "Queued" => OperationState::Queued,
        "Sending" => OperationState::Sending,
        "Sent" => OperationState::Sent,
        "Failed" => OperationState::Failed,
        "Unknown" => OperationState::Unknown,
        _ => return Err(OutgoingError::InvalidData),
    };
    let id =
        Uuid::from_slice(&row.get::<_, Vec<u8>>(0)?).map_err(|_| OutgoingError::InvalidData)?;
    let payload: Vec<u8> = row.get(2)?;
    let mut kind: OperationKind = decode(&payload)?;
    let retained = previous
        .get(&id)
        .filter(|op| op.state != OperationState::Sent && state != OperationState::Sent);
    if retained.is_none()
        && state != OperationState::Sent
        && let OperationKind::Message(message) = &mut kind
    {
        let references = row
            .get::<_, Option<Vec<u8>>>(9)?
            .map_or_else(|| Ok(Vec::new()), |v| decode(&v))?;
        files::load(root, account, &mut message.attachments, &references)?;
    }
    Ok(Operation {
        id,
        channel_id: Uuid::from_slice(&row.get::<_, Vec<u8>>(1)?)
            .map_err(|_| OutgoingError::InvalidData)?,
        kind: retained.map_or_else(|| Arc::new(kind), |op| Arc::clone(&op.kind)),
        state,
        attempts: row.get(4)?,
        depends_on: row
            .get::<_, Option<Vec<u8>>>(5)?
            .map(|id| Uuid::from_slice(&id).map_err(|_| OutgoingError::InvalidData))
            .transpose()?,
        failure: row
            .get::<_, Option<Vec<u8>>>(6)?
            .map(|v| decode(&v))
            .transpose()?,
        message_id: outcome.message_id,
        history_projected: outcome.projected,
        message: outcome.message.map(Arc::new),
        created_at_us: row.get(8)?,
    })
}
impl Registry {
    pub(super) async fn outgoing_projected(
        &self,
        session: Session,
        ids: Vec<Uuid>,
    ) -> Result<(), OutgoingError> {
        self.writer.write(move |tx| {
            visible(tx,&session)?;
            let account=session.account();
            for id in ids {tx.execute("UPDATE operations SET result=CAST(json_set(result,'$.projected',json('true')) AS BLOB) WHERE local_id=?1 AND server_id=?2 AND user_id=?3 AND state='Sent'",params![id.as_bytes(),account.server().as_bytes(),account.user().as_bytes()])?;}
            Ok(())
        }).await
    }

    pub(super) async fn outgoing_list(
        &self,
        session: Session,
        previous: Arc<super::OutgoingSnapshot>,
    ) -> Result<Vec<Arc<Operation>>, OutgoingError> {
        let root = self
            .path
            .parent()
            .ok_or(OutgoingError::InvalidData)?
            .to_path_buf();
        let lease = Arc::clone(&self.lease);
        let previous: std::collections::BTreeMap<_, _> = previous
            .operations
            .iter()
            .map(|op| (op.id, Arc::clone(op)))
            .collect();
        self.readers.query(self.path.clone(), move |tx| {
            let _lease = lease;
            Ok((|| { visible(tx, &session)?;
            let id = session.account();
            let mut statement = tx.prepare("SELECT local_id,channel_id,payload,state,attempts,depends_on,last_error,result,created_at_us,files FROM operations WHERE server_id=?1 AND user_id=?2 AND discarded=0 ORDER BY rowid")?;
            let mut rows = statement.query(params![id.server().as_bytes(), id.user().as_bytes()])?;
            let mut values = Vec::new();
            while let Some(row) = rows.next()? { values.push(Arc::new(read_operation(row, &root, id, &previous)?)); }
            Ok(values) })())
        }).await.map_err(OutgoingError::from)?
    }

    pub(super) async fn outgoing_enqueue(
        &self,
        session: Session,
        channel: Uuid,
        local: Uuid,
        kind: OperationKind,
        created_at_us: i64,
        draft_revision: Option<u64>,
    ) -> Result<(), OutgoingError> {
        let mut kind = if let Some(revision) = draft_revision {
            let draft = self
                .draft_load(session.clone(), channel)
                .await?
                .ok_or(OutgoingError::StaleDraft)?;
            if draft.revision != revision {
                return Err(OutgoingError::StaleDraft);
            }
            OperationKind::Message(draft.body.message)
        } else {
            kind
        };
        kind.validate()?;
        let bytes = if let OperationKind::Message(message) = &mut kind {
            files::detach(&mut message.attachments)
        } else {
            Vec::new()
        };
        let (references, attachment_bytes) = self.stage_files(&session, bytes, local).await?;
        let payload = encode(&kind)?;
        self.writer.write(move |tx| {
            visible(tx, &session)?;
            let account = session.account();
            if let Some(expected) = draft_revision {
                let current: i64 = tx.query_row("SELECT revision FROM drafts WHERE server_id=?1 AND user_id=?2 AND channel_id=?3",params![account.server().as_bytes(),account.user().as_bytes(),channel.as_bytes()],|row|row.get(0))?;
                if u64::try_from(current).ok()!=Some(expected) {return Err(OutgoingError::StaleDraft);}
            }
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE local_id=?1)", [local.as_bytes()], |row|row.get(0))?;
            if exists { return Err(OutgoingError::AlreadyExists); }
            let (count, retained_bytes):(i64,i64) = tx.query_row("SELECT count(*),coalesce(sum(length(payload)+attachment_bytes),0) FROM operations WHERE server_id=?1 AND user_id=?2 AND discarded=0 AND (state!='Sent' OR coalesce(json_extract(result,'$.projected'),0)=0)",params![account.server().as_bytes(),account.user().as_bytes()], |row|Ok((row.get(0)?,row.get(1)?)))?;
            if count >= 256 || retained_bytes.saturating_add(i64::try_from(payload.len()).map_err(|_|OutgoingError::Budget)?).saturating_add(attachment_bytes) > 256*1024*1024 { return Err(OutgoingError::Budget); }
            if let Some(dependency) = kind.dependency() {
                let data: Option<Vec<u8>> = tx.query_row("SELECT payload FROM operations WHERE local_id=?1 AND server_id=?2 AND user_id=?3 AND channel_id=?4 AND discarded=0",params![dependency.as_bytes(),account.server().as_bytes(),account.user().as_bytes(),channel.as_bytes()], |row|row.get(0)).optional()?;
                if !matches!(data.map(|v|decode::<OperationKind>(&v)).transpose()?, Some(OperationKind::Message(_))) { return Err(OutgoingError::NotFound); }
            }
            tx.execute("INSERT INTO operations(local_id,server_id,user_id,channel_id,kind,payload,depends_on,state,created_at_us,draft_revision,files,attachment_bytes) VALUES(?1,?2,?3,?4,'sdk-v1',?5,?6,'Queued',?7,?8,?9,?10)",params![local.as_bytes(),account.server().as_bytes(),account.user().as_bytes(),channel.as_bytes(),payload,kind.dependency().map(|id|id.as_bytes().to_vec()),created_at_us,draft_revision.map(i64::try_from).transpose().map_err(|_|OutgoingError::InvalidData)?,encode(&references)?,attachment_bytes])?;
            Ok(())
        }).await
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
        self.writer.write(move |tx| {
            visible(tx, &session)?;
            let account = session.account();
            let changed = tx.execute("UPDATE operations SET state=?1,attempts=attempts+?2,result=?3,last_error=?4 WHERE local_id=?5 AND server_id=?6 AND user_id=?7 AND state=?8 AND discarded=0",params![format!("{next:?}"),u32::from(next==OperationState::Sending),outcome.map(|v|encode(&v)).transpose()?,failure.map(|v|encode(&v)).transpose()?,local.as_bytes(),account.server().as_bytes(),account.user().as_bytes(),format!("{expected:?}")])?;
            if changed != 1 { return Err(OutgoingError::InvalidState); }
            if next == OperationState::Sent {
                let bytes:Vec<u8>=tx.query_row("SELECT payload FROM operations WHERE local_id=?1",[local.as_bytes()],|row|row.get(0))?;
                let mut kind:OperationKind=decode(&bytes)?;
                if let OperationKind::Message(message)=&mut kind {
                    for file in &mut message.attachments {file.bytes.clear();}
                }
                let order:i64=tx.query_row("SELECT coalesce(max(confirmed_order),0) FROM operations",[],|row|row.get(0))?;
                let order=order.checked_add(1).ok_or(OutgoingError::InvalidData)?;
                tx.execute("UPDATE operations SET payload=?1,depends_on=NULL,confirmed_order=?3,files=NULL,attachment_bytes=0 WHERE local_id=?2",params![encode(&kind)?,local.as_bytes(),order])?;
                // Only confirmed receipts are compacted; unresolved intent and required mappings survive.
                tx.execute("DELETE FROM operations WHERE server_id=?1 AND user_id=?2 AND ((state='Sent' AND json_extract(result,'$.projected')=1) OR discarded=1) AND NOT EXISTS(SELECT 1 FROM operations child WHERE child.depends_on=operations.local_id) AND rowid NOT IN (SELECT rowid FROM operations WHERE server_id=?1 AND user_id=?2 AND state='Sent' AND discarded=0 ORDER BY confirmed_order DESC LIMIT 256)",params![account.server().as_bytes(),account.user().as_bytes()])?;
                // Clear only the captured revision. A newer draft is unrelated device-owned work.
                tx.execute("UPDATE drafts SET body=NULL,files=NULL,text='',mentions=x'',reply_to=NULL,revision=revision+1 WHERE server_id=?1 AND user_id=?2 AND (channel_id,revision) IN (SELECT channel_id,draft_revision FROM operations WHERE local_id=?3)",params![account.server().as_bytes(),account.user().as_bytes(),local.as_bytes()])?;
            }
            Ok(())
        }).await
    }

    pub(super) async fn outgoing_discard(
        &self,
        session: Session,
        local: Uuid,
    ) -> Result<(), OutgoingError> {
        self.writer.write(move |tx| {
            visible(tx, &session)?;
            let a=session.account();
            let has_dependents:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE depends_on=?1 AND discarded=0 AND state!='Sent')",[local.as_bytes()],|row|row.get(0))?;
            let sent:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE local_id=?1 AND server_id=?2 AND user_id=?3 AND state='Sent')",params![local.as_bytes(),a.server().as_bytes(),a.user().as_bytes()],|row|row.get(0))?;
            if sent && has_dependents {return Err(OutgoingError::InvalidState);}
            if tx.execute("UPDATE operations SET discarded=1 WHERE local_id=?1 AND server_id=?2 AND user_id=?3 AND state!='Sending' AND discarded=0",params![local.as_bytes(),a.server().as_bytes(),a.user().as_bytes()])? != 1 { return Err(OutgoingError::InvalidState); }
            tx.execute("UPDATE operations SET state='Failed',last_error=?1 WHERE depends_on=?2 AND server_id=?3 AND user_id=?4 AND state='Queued'",params![encode(&OperationFailure::Dependency)?,local.as_bytes(),a.server().as_bytes(),a.user().as_bytes()])?;
            tx.execute("UPDATE operations SET payload=x'',result=NULL,files=NULL,attachment_bytes=0 WHERE local_id=?1 AND discarded=1",[local.as_bytes()])?;
            tx.execute("DELETE FROM operations WHERE server_id=?1 AND user_id=?2 AND discarded=1 AND NOT EXISTS(SELECT 1 FROM operations child WHERE child.depends_on=operations.local_id) AND rowid NOT IN(SELECT rowid FROM operations WHERE server_id=?1 AND user_id=?2 AND discarded=1 ORDER BY rowid DESC LIMIT 256)",params![a.server().as_bytes(),a.user().as_bytes()])?;
            Ok(())
        }).await
    }

    pub(super) async fn draft_load(
        &self,
        session: Session,
        channel: Uuid,
    ) -> Result<Option<Draft>, OutgoingError> {
        let root = self
            .path
            .parent()
            .ok_or(OutgoingError::InvalidData)?
            .to_path_buf();
        let lease = Arc::clone(&self.lease);
        self.readers
            .query(self.path.clone(), move |tx| {
                let _lease = lease;
                Ok((|| {
                    visible(tx, &session)?;
                    load_draft(tx, session.account(), channel, &root)
                })())
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
        let mut kind = OperationKind::Message(body.message);
        kind.validate()?;
        let bytes = if let OperationKind::Message(message) = &mut kind {
            files::detach(&mut message.attachments)
        } else {
            Vec::new()
        };
        let OperationKind::Message(message) = kind else {
            return Err(OutgoingError::InvalidData);
        };
        body.message = message;
        let (references, _) = self.stage_files(&session, bytes, channel).await?;
        self.writer.write(move |tx| {
            visible(tx,&session)?;
            let a=session.account();
            let revision:i64=tx.query_row("SELECT revision FROM drafts WHERE server_id=?1 AND user_id=?2 AND channel_id=?3",params![a.server().as_bytes(),a.user().as_bytes(),channel.as_bytes()],|r|r.get(0)).optional()?.unwrap_or(0);
            if expected.is_some_and(|expected|i64::try_from(expected).ok()!=Some(revision)) { return Err(OutgoingError::StaleDraft); }
            let revision=revision.checked_add(1).ok_or(OutgoingError::InvalidData)?;
            tx.execute("INSERT INTO drafts(server_id,user_id,channel_id,text,mentions,updated_at_us,body,revision,files) VALUES(?1,?2,?3,'',x'',?4,?5,?6,?7) ON CONFLICT(server_id,user_id,channel_id) DO UPDATE SET body=excluded.body,files=excluded.files,revision=excluded.revision,updated_at_us=excluded.updated_at_us",params![a.server().as_bytes(),a.user().as_bytes(),channel.as_bytes(),updated_at_us,encode(&body)?,revision,encode(&references)?])?;
            u64::try_from(revision).map_err(|_|OutgoingError::InvalidData)
        }).await
    }
}
fn load_draft(
    tx: &Transaction<'_>,
    a: AccountId,
    channel: Uuid,
    root: &Path,
) -> Result<Option<Draft>, OutgoingError> {
    type DraftRow = (i64, Option<Vec<u8>>, i64, Option<Vec<u8>>);
    let row:Option<DraftRow>=tx.query_row("SELECT revision,body,updated_at_us,files FROM drafts WHERE server_id=?1 AND user_id=?2 AND channel_id=?3",params![a.server().as_bytes(),a.user().as_bytes(),channel.as_bytes()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    row.map(|(revision, body, updated_at_us, references)| {
        let mut body: DraftBody = body.map_or_else(|| Ok(DraftBody::default()), |v| decode(&v))?;
        let references = references.map_or_else(|| Ok(Vec::new()), |v| decode(&v))?;
        files::load(root, a, &mut body.message.attachments, &references)?;
        Ok(Draft {
            revision: u64::try_from(revision).map_err(|_| OutgoingError::InvalidData)?,
            body,
            updated_at_us,
        })
    })
    .transpose()
}
