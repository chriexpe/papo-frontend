//! Messages sent while one of their files is still being copied into staging.
//!
//! Owns committing such a message at once, showing it as Preparing, and resolving it into an
//! ordinary Queued operation when every file is ready. Leaves the copy itself to `pending`,
//! edits to `image_edit`, staging checks to `file_io` and dispatch to `dispatch`.
//!
//! The row never says Preparing (the schema's state check has no such value). It is written as
//! what the message becomes if the process ends before its files are ready: Failed with
//! [`OperationFailure::AttachmentLost`], no file references. While this process copies, the
//! queue's `preparing` map shows it as Preparing. Resolving writes the file references, the
//! cleaned payload and Queued in one transaction, so every other part of the queue only ever
//! sees a row with all of its files or none.

mod overlay;

use super::{
    OperationFailure, OperationKind, OperationState, OutgoingAttachmentContent, OutgoingError,
    OutgoingMessage, OutgoingService, PendingAttachment, StagedAttachment, StagingFailure, Uuid,
    enqueue::{self, MAX_RETAINED_BYTES},
    files,
    persistence::{encode, visible},
};
use crate::{Registry, Session};
use rusqlite::params;
use tokio_util::sync::CancellationToken;

const INSERT_PREPARING_SQL: &str = "
    INSERT INTO operations (
        local_id, server_id, user_id, channel_id, kind, payload, depends_on, state,
        created_at_us, last_error, attachment_bytes
    )
    VALUES (?1, ?2, ?3, ?4, 'sdk-v1', ?5, ?6, 'Failed', ?7, ?8, 0)";

const PREPARING_ROW_SQL: &str = "
    SELECT length(payload) FROM operations
    WHERE local_id = ?1 AND server_id = ?2 AND user_id = ?3
      AND state = 'Failed' AND last_error = ?4 AND discarded = 0";

const RESOLVE_SQL: &str = "
    UPDATE operations
    SET payload = ?1, files = ?2, attachment_bytes = ?3, state = ?4, last_error = ?5
    WHERE local_id = ?6 AND server_id = ?7 AND user_id = ?8
      AND state = 'Failed' AND last_error = ?9 AND discarded = 0";

const RETAINED_BYTES_SQL: &str = "
    SELECT coalesce(sum(length(payload) + attachment_bytes), 0)
    FROM operations
    WHERE server_id = ?1 AND user_id = ?2 AND discarded = 0
      AND (state != 'Sent' OR coalesce(json_extract(result, '$.projected'), 0) = 0)";

/// One preparing message in this process.
pub(super) struct Preparing {
    /// ✕ was pressed: shown Failed (Cancelled) while the copy keeps running, so Retry can send it.
    cancelled: bool,
    /// Stops waiting for the copy (Discard).
    abort: CancellationToken,
}

/// A file of a preparing message: already staged, or still copying.
enum Slot {
    Ready(StagedAttachment),
    Copying(PendingAttachment),
}

/// Files staged and the message rewritten, ready to become Queued.
#[derive(Clone)]
struct Resolved {
    payload: Vec<u8>,
    references: Vec<files::Reference>,
    attachment_bytes: i64,
    /// Keeps the staged blobs pinned until the transaction that references them has committed.
    _held: Vec<StagedAttachment>,
}

fn lost() -> Result<Vec<u8>, OutgoingError> {
    encode(&OperationFailure::AttachmentLost)
}

/// True when `message` has a file still copying, so its enqueue takes the preparing path.
pub(super) fn has_copying(message: &OutgoingMessage) -> bool {
    message
        .attachments
        .iter()
        .any(|attachment| attachment.pending().is_some())
}

impl Registry {
    /// Commits a message whose files are still copying, then resolves it in the background.
    /// Runs under the queue's serial lock, like every enqueue; nothing here waits for a copy.
    pub(super) async fn outgoing_enqueue_preparing(
        &self,
        session: Session,
        channel: Uuid,
        local: Uuid,
        mut kind: OperationKind,
        created_at_us: i64,
    ) -> Result<(), OutgoingError> {
        let OperationKind::Message(message) = &mut kind else {
            return Err(OutgoingError::InvalidInput);
        };
        let account = session.account();
        let mut slots = Vec::with_capacity(message.attachments.len());
        for attachment in &mut message.attachments {
            slots.push(match attachment.take_content() {
                OutgoingAttachmentContent::Staged(staged) if staged.account() == account => {
                    Slot::Ready(staged)
                }
                OutgoingAttachmentContent::Pending(pending) => Slot::Copying(pending),
                _ => return Err(OutgoingError::InvalidInput),
            });
        }
        let payload = encode(&kind)?;
        let dependency = kind.dependency();
        let failure = lost()?;
        let abort = CancellationToken::new();
        // Shown as Preparing from its first publication.
        session.inner.outgoing.preparing_map().insert(
            local,
            Preparing {
                cancelled: false,
                abort: abort.clone(),
            },
        );
        let committed = self
            .writer
            .write({
                let session = session.clone();
                move |tx| {
                    visible(tx, &session)?;
                    enqueue::ensure_unique_local_id(tx, local)?;
                    enqueue::check_admission_budget(tx, account, payload.len(), 0)?;
                    if let Some(dependency_id) = dependency {
                        enqueue::verify_dependency(tx, account, channel, dependency_id)?;
                    }
                    tx.execute(
                        INSERT_PREPARING_SQL,
                        params![
                            local.as_bytes(),
                            account.server().as_bytes(),
                            account.user().as_bytes(),
                            channel.as_bytes(),
                            payload,
                            dependency.map(|id| id.as_bytes().to_vec()),
                            created_at_us,
                            failure,
                        ],
                    )?;
                    Ok::<_, OutgoingError>(())
                }
            })
            .await;
        if let Err(error) = committed {
            session.inner.outgoing.preparing_map().remove(&local);
            return Err(error);
        }
        let OperationKind::Message(committed_message) = kind else {
            return Err(OutgoingError::InvalidInput);
        };
        let token = session.inner.tasks.token();
        let service = OutgoingService { session };
        let runtime = service.session.client.inner.runtime.clone();
        runtime.spawn(async move {
            let _token = token;
            service
                .resolve_preparing(local, committed_message, slots, abort)
                .await;
        });
        Ok(())
    }
}

impl OutgoingService {
    /// `message` is the committed one (files not attached), never read back from the snapshot:
    /// the copy may end before the enqueue that committed it has published.
    async fn resolve_preparing(
        &self,
        local: Uuid,
        message: OutgoingMessage,
        slots: Vec<Slot>,
        abort: CancellationToken,
    ) {
        let outcome = match self.gather(slots, &abort).await {
            Some(Ok(files)) => self.prepare(local, message, files).await,
            Some(Err(failure)) => Err(failure),
            // Discarded, or the account closed: the row already says what it is.
            None => return,
        };
        self.finish_preparing(local, outcome).await;
    }

    /// Waits for every copy; `None` when waiting itself was abandoned.
    async fn gather(
        &self,
        slots: Vec<Slot>,
        abort: &CancellationToken,
    ) -> Option<Result<Vec<StagedAttachment>, OperationFailure>> {
        let mut files = Vec::with_capacity(slots.len());
        for slot in slots {
            let staged = match slot {
                Slot::Ready(staged) => Ok(staged),
                Slot::Copying(pending) => tokio::select! {
                    biased;
                    () = abort.cancelled() => return None,
                    () = self.session.inner.root.cancelled() => return None,
                    result = pending.staged() => result,
                },
            };
            match staged {
                Ok(staged) => files.push(staged),
                Err(StagingFailure::TooLarge) => {
                    return Some(Err(OperationFailure::AttachmentRefused));
                }
                Err(_) => return Some(Err(OperationFailure::AttachmentLost)),
            }
        }
        Some(Ok(files))
    }

    /// Rewrites the committed message with its staged files and edits, as an enqueue would.
    async fn prepare(
        &self,
        local: Uuid,
        mut message: OutgoingMessage,
        files: Vec<StagedAttachment>,
    ) -> Result<Resolved, OperationFailure> {
        if message.attachments.len() != files.len() {
            return Err(OperationFailure::AttachmentLost);
        }
        for (attachment, staged) in message.attachments.iter_mut().zip(&files) {
            attachment.set_staged(staged.clone());
        }
        let registry = &self.session.client.inner.registry;
        registry
            .outgoing_apply_edits(&self.session, &mut message, local)
            .await
            .map_err(|error| match error {
                OutgoingError::Image(_) | OutgoingError::InvalidInput => {
                    OperationFailure::AttachmentRefused
                }
                _ => OperationFailure::AttachmentLost,
            })?;
        message
            .validate()
            .map_err(|_| OperationFailure::AttachmentRefused)?;
        let held: Vec<StagedAttachment> = message
            .attachments
            .iter()
            .filter_map(|attachment| attachment.staged().cloned())
            .collect();
        let inputs = files::detach_inputs(&mut message.attachments, self.session.account())
            .map_err(|_| OperationFailure::AttachmentLost)?;
        let (references, attachment_bytes) = registry
            .prepare_attachments(&self.session, inputs, local)
            .await
            .map_err(|_| OperationFailure::AttachmentLost)?;
        let payload = encode(&OperationKind::Message(message))
            .map_err(|_| OperationFailure::AttachmentLost)?;
        Ok(Resolved {
            payload,
            references,
            attachment_bytes,
            _held: held,
        })
    }

    /// Records the outcome under the queue's serial lock, then republishes and wakes dispatch.
    ///
    /// A write that fails is tried again with the client's backoff while the account is open:
    /// the files are staged, so giving up would lose a message that only needs its row written.
    /// The entry stays (shown Preparing) until the write lands, or the message is discarded.
    async fn finish_preparing(&self, local: Uuid, outcome: Result<Resolved, OperationFailure>) {
        let queue = &self.session.inner.outgoing;
        let config = &self.session.client.inner.config;
        let mut attempt = 0_u32;
        loop {
            let guard = queue.serial.lock().await;
            let Some(cancelled) = queue
                .preparing_map()
                .get(&local)
                .map(|entry| entry.cancelled)
            else {
                return; // Discarded meanwhile.
            };
            if self
                .write_outcome(local, cancelled, outcome.clone())
                .await
                .is_ok()
            {
                queue.preparing_map().remove(&local);
                let _ = self.publish().await;
                queue.wake.notify_one();
                return;
            }
            drop(guard);
            let delay = config.backoff.delay(attempt, config.jitter.as_ref());
            attempt = attempt.saturating_add(1);
            tokio::select! {
                biased;
                () = self.session.inner.root.cancelled() => return,
                () = tokio::time::sleep(delay) => {}
            }
        }
    }

    async fn write_outcome(
        &self,
        local: Uuid,
        cancelled: bool,
        outcome: Result<Resolved, OperationFailure>,
    ) -> Result<(), OutgoingError> {
        let session = self.session.clone();
        let lost = lost()?;
        let account = session.account();
        let registry = &self.session.client.inner.registry;
        registry
            .writer
            .write(move |tx| {
                visible(tx, &session)?;
                let keys = (
                    account.server().as_bytes().to_vec(),
                    account.user().as_bytes().to_vec(),
                );
                let previous_payload_bytes: Option<i64> =
                    rusqlite::OptionalExtension::optional(tx.query_row(
                        PREPARING_ROW_SQL,
                        params![local.as_bytes(), keys.0, keys.1, lost],
                        |row| row.get(0),
                    ))?;
                let Some(previous_payload_bytes) = previous_payload_bytes else {
                    return Ok(()); // Discarded or no longer preparing.
                };
                let failed = |failure: OperationFailure| -> Result<(), OutgoingError> {
                    tx.execute(
                        "UPDATE operations SET last_error = ?1 WHERE local_id = ?2",
                        params![encode(&failure)?, local.as_bytes()],
                    )?;
                    Ok(())
                };
                let resolved = match outcome {
                    Ok(resolved) => resolved,
                    Err(failure) => return failed(failure),
                };
                let retained_bytes: i64 =
                    tx.query_row(RETAINED_BYTES_SQL, params![keys.0, keys.1], |row| {
                        row.get(0)
                    })?;
                let added_bytes = i64::try_from(resolved.payload.len())
                    .map_err(|_| OutgoingError::Budget)?
                    .saturating_sub(previous_payload_bytes)
                    .saturating_add(resolved.attachment_bytes);
                if retained_bytes.saturating_add(added_bytes) > MAX_RETAINED_BYTES {
                    return failed(OperationFailure::AttachmentRefused);
                }
                let (state, failure) = if cancelled {
                    (
                        OperationState::Failed,
                        Some(encode(&OperationFailure::Cancelled)?),
                    )
                } else {
                    (OperationState::Queued, None)
                };
                tx.execute(
                    RESOLVE_SQL,
                    params![
                        resolved.payload,
                        encode(&resolved.references)?,
                        resolved.attachment_bytes,
                        state.as_str(),
                        failure,
                        local.as_bytes(),
                        keys.0,
                        keys.1,
                        lost,
                    ],
                )?;
                Ok(())
            })
            .await
    }
}

/// A row stored as preparing (or failed while preparing) carries no file references; it loads
/// with its files unattached.
pub(super) fn stored_without_files(failure: Option<OperationFailure>) -> bool {
    matches!(
        failure,
        Some(OperationFailure::AttachmentLost | OperationFailure::AttachmentRefused)
    )
}
