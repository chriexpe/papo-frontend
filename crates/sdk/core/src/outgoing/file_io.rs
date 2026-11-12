//! Bounded asynchronous attachment I/O; only the short reference snapshot enters the SQLite writer.

use super::{OutgoingError, Uuid, files, persistence::visible};
use crate::{Registry, Session};

impl Registry {
    pub(super) async fn outgoing_collect(&self, session: Session) -> Result<(), OutgoingError> {
        let root = self
            .path
            .parent()
            .ok_or(OutgoingError::InvalidData)?
            .to_path_buf();
        let guard = self
            .file_guard(&session, session.client.inner.outgoing_collection.clone())
            .await?;
        let account = session.account();
        let names = self
            .writer
            .write(move |tx| {
                visible(tx, &session)?;
                Ok::<_, OutgoingError>(files::referenced_names(account, tx)?)
            })
            .await?;
        let lease = self.lease.clone();
        tokio::task::spawn_blocking(move || {
            let (_guard, _lease) = (guard, lease);
            files::collect(&root, account, &names)
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
            tokio::sync::OwnedMutexGuard<()>,
            tokio::sync::OwnedSemaphorePermit,
        ),
        OutgoingError,
    > {
        let guard = session
            .inner
            .outgoing
            .files_serial
            .clone()
            .lock_owned()
            .await;
        let permit = pool
            .acquire_owned()
            .await
            .map_err(|_| OutgoingError::InvalidState)?;
        Ok((guard, permit))
    }

    pub(super) async fn stage_files(
        &self,
        session: &Session,
        bytes: Vec<Vec<u8>>,
        identity: Uuid,
    ) -> Result<(Vec<files::Reference>, i64), OutgoingError> {
        if bytes.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let guard = self
            .file_guard(session, session.client.inner.outgoing_files.clone())
            .await?;
        let root = self
            .path
            .parent()
            .ok_or(OutgoingError::InvalidData)?
            .to_path_buf();
        let session = session.clone();
        let lease = self.lease.clone();
        let runtime = session.client.inner.runtime.clone();
        tokio::task::spawn_blocking(move || {
            let (_guard, _lease) = (guard, lease);
            if session.is_closed() {
                return Err(OutgoingError::InvalidState);
            }
            let size = files::size(&bytes)?;
            let mut notified = false;
            let refs = files::stage_observed(&root, session.account(), &bytes, || {
                if !notified {
                    notified = true;
                    #[cfg(feature = "test-support")]
                    if let Some(probe) = &session.client.inner.config.outgoing_probe {
                        runtime.block_on(
                            probe.reached(identity, super::OutgoingDispatchPhase::Staging),
                        );
                    }
                }
            })?;
            #[cfg(not(feature = "test-support"))]
            let _ = (identity, runtime);
            Ok((refs, size))
        })
        .await
        .map_err(|_| OutgoingError::InvalidData)?
    }
}
