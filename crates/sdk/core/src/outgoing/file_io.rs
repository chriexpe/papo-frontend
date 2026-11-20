//! Bounded asynchronous attachment I/O; only short reference snapshots enter the SQLite writer.

use super::{
    ClientError, OutgoingError, StagedAttachment, Uuid,
    files::{self, AttachmentInput, Reference},
    persistence::{data_root, visible},
    stream::OpenedAttachment,
};
use crate::{Registry, Session};
use std::io::{Cursor, Read};

impl Registry {
    pub(super) async fn outgoing_collect(&self, session: Session) -> Result<(), OutgoingError> {
        let root = data_root(self)?;
        let guard = self
            .file_guard(&session, session.client.inner.outgoing_collection.clone())
            .await?;
        let account = session.account();
        let mut names = self
            .writer
            .write({
                let session = session.clone();
                move |tx| {
                    visible(tx, &session)?;
                    Ok::<_, OutgoingError>(files::referenced_names(account, tx)?)
                }
            })
            .await?;
        names.extend(session.inner.outgoing.pinned_names());
        let token = session.inner.tasks.token();
        if session.is_closed() {
            return Err(ClientError::Closed.into());
        }
        let lease = self.lease.clone();
        tokio::task::spawn_blocking(move || {
            let (_guard, _token, _lease) = (guard, token, lease);
            if session.is_closed() {
                return Err(OutgoingError::Session(ClientError::Closed));
            }
            files::collect(&root, account, &names).map_err(OutgoingError::from)
        })
        .await
        .map_err(|_| OutgoingError::InvalidData)??;
        Ok(())
    }

    async fn file_guard(
        &self,
        session: &Session,
        pool: std::sync::Arc<tokio::sync::Semaphore>,
    ) -> Result<
        (
            tokio::sync::OwnedSemaphorePermit,
            tokio::sync::OwnedMutexGuard<()>,
        ),
        OutgoingError,
    > {
        // Acquire the profile-wide permit before locking this account's files_serial so an
        // unrelated account's bulk copies cannot stall SessionInner::stop() during removal.
        let permit = tokio::select! {
            biased;
            () = session.inner.root.cancelled() => return Err(ClientError::Closed.into()),
            permit = pool.acquire_owned() => permit.map_err(|_| OutgoingError::InvalidState)?,
        };
        let guard = tokio::select! {
            biased;
            () = session.inner.root.cancelled() => return Err(ClientError::Closed.into()),
            guard = session.inner.outgoing.files_serial.clone().lock_owned() => guard,
        };
        if session.is_closed() {
            return Err(ClientError::Closed.into());
        }
        Ok((permit, guard))
    }

    /// Streams one caller-supplied reader into this account's durable content-addressed staging
    /// on a bounded blocking task outside the SQLite writer.
    pub(super) async fn stage_reader<R: Read + Send + 'static>(
        &self,
        session: &Session,
        reader: R,
        identity: Uuid,
    ) -> Result<StagedAttachment, OutgoingError> {
        let guard = self
            .file_guard(session, session.client.inner.outgoing_files.clone())
            .await?;
        let token = session.inner.tasks.token();
        if session.is_closed() {
            return Err(ClientError::Closed.into());
        }
        let root = data_root(self)?;
        let session = session.clone();
        let lease = self.lease.clone();
        let account = session.account();
        tokio::task::spawn_blocking(move || {
            let (_guard, _token, _lease) = (guard, token, lease);
            let mut notified = false;
            let reference = files::stage_reader(
                &root,
                account,
                reader,
                || session.is_closed(),
                || {
                    if !notified {
                        notified = true;
                        notify_staging_probe(&session, identity);
                    }
                },
            )?;
            // Pin before releasing files_serial so background collection cannot unlink the blob
            // while the caller still holds the returned StagedAttachment.
            let pin = session
                .inner
                .outgoing
                .pin_staged(files::digest_hex(&reference.digest));
            Ok(StagedAttachment::with_pin(
                account,
                reference.digest,
                reference.size_bytes,
                pin,
            ))
        })
        .await
        .map_err(|_| OutgoingError::InvalidData)?
    }

    /// Stages inline caller bytes through the shared streaming writer and verifies already-staged
    /// references in exact attachment order.
    pub(super) async fn prepare_attachments(
        &self,
        session: &Session,
        inputs: Vec<AttachmentInput>,
        identity: Uuid,
    ) -> Result<(Vec<Reference>, i64), OutgoingError> {
        if inputs.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let guard = self
            .file_guard(session, session.client.inner.outgoing_files.clone())
            .await?;
        let token = session.inner.tasks.token();
        if session.is_closed() {
            return Err(ClientError::Closed.into());
        }
        let root = data_root(self)?;
        let session = session.clone();
        let lease = self.lease.clone();
        let account = session.account();
        tokio::task::spawn_blocking(move || {
            let (_guard, _token, _lease) = (guard, token, lease);
            if session.is_closed() {
                return Err(ClientError::Closed.into());
            }
            let mut notified = false;
            let mut refs = Vec::with_capacity(inputs.len());
            for input in inputs {
                match input {
                    AttachmentInput::Inline(bytes) => {
                        let reference = files::stage_reader(
                            &root,
                            account,
                            Cursor::new(bytes),
                            || session.is_closed(),
                            || {
                                if !notified {
                                    notified = true;
                                    notify_staging_probe(&session, identity);
                                }
                            },
                        )?;
                        refs.push(reference);
                    }
                    AttachmentInput::Staged(reference) => {
                        files::verify_staged_metadata(&root, account, reference)?;
                        refs.push(reference);
                    }
                }
            }
            let total_bytes = files::total_reference_bytes(&refs)?;
            Ok((refs, total_bytes))
        })
        .await
        .map_err(|_| OutgoingError::InvalidData)?
    }

    /// Streams SHA-256 verification of staged files outside any SQLite read transaction,
    /// tracked on `session.inner.tasks` so account removal drains open file handles first.
    pub(super) async fn verify_staged_files(
        &self,
        session: &Session,
        refs: Vec<Reference>,
    ) -> Result<(), OutgoingError> {
        if refs.is_empty() {
            return Ok(());
        }
        let token = session.inner.tasks.token();
        if session.is_closed() {
            return Err(ClientError::Closed.into());
        }
        let root = data_root(self)?;
        let session = session.clone();
        let account = session.account();
        let lease = self.lease.clone();
        tokio::task::spawn_blocking(move || {
            let (_token, _lease) = (token, lease);
            files::verify(&root, account, &refs, || session.is_closed())
        })
        .await
        .map_err(|_| OutgoingError::InvalidData)?
    }

    /// Opens validated staged files owned by `session` before committing `Sending`,
    /// tracked on `session.inner.tasks` so account removal cannot race open handles.
    pub(super) async fn open_upload_files(
        &self,
        session: &Session,
        items: Vec<(String, Reference)>,
    ) -> Result<Vec<OpenedAttachment>, OutgoingError> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let token = session.inner.tasks.token();
        if session.is_closed() {
            return Err(ClientError::Closed.into());
        }
        let root = data_root(self)?;
        let session = session.clone();
        let account = session.account();
        let lease = self.lease.clone();
        tokio::task::spawn_blocking(move || {
            let (_token, _lease) = (token, lease);
            if session.is_closed() {
                return Err(OutgoingError::Session(ClientError::Closed));
            }
            let mut opened = Vec::with_capacity(items.len());
            for (filename, reference) in items {
                let file = files::open_for_upload(&root, account, reference)?;
                opened.push(OpenedAttachment {
                    filename,
                    reference,
                    file,
                });
            }
            Ok(opened)
        })
        .await
        .map_err(|_| OutgoingError::InvalidData)?
    }
}

#[cfg(feature = "test-support")]
fn notify_staging_probe(session: &Session, identity: Uuid) {
    if let Some(probe) = &session.client.inner.config.outgoing_probe {
        session
            .client
            .inner
            .runtime
            .block_on(probe.reached(identity, super::OutgoingDispatchPhase::Staging));
    }
}

#[cfg(not(feature = "test-support"))]
fn notify_staging_probe(_session: &Session, _identity: Uuid) {}
