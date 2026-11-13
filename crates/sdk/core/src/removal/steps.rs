//! Idempotent cleanup steps; the journal alone acknowledges their completion.

use super::{
    RemovalFailureKind as Failure, RemovalJob, RemovalJobs, RemovalStep as Step, RemovalTarget,
};
use crate::secret::{SecretError, SecretFailure};
use crate::{AccountId, StorageError};

impl RemovalJobs {
    pub(super) async fn run(&self, mut job: RemovalJob) -> Result<(), StorageError> {
        let RemovalTarget::Account(account) = job.target else {
            return self.finalize(job).await;
        };
        let Some(key) = job.key else {
            return Err(StorageError::InvalidState);
        };
        if let Err(error) = self.secrets.activate(account, key.generation()) {
            self.mark(job.id, job.step, Some(secret_failure(error)))
                .await?;
            return Ok(());
        }
        loop {
            if self.cancel.is_cancelled() {
                self.mark(job.id, job.step, Some(Failure::Canceled)).await?;
                return Ok(());
            }
            let result = match job.step {
                Step::StopSession => tokio::select! {
                    () = self.cancel.cancelled() => Err(Failure::Canceled),
                    result = self.hooks.stop_session(account) => result.map_err(|_|Failure::SessionStop),
                },
                Step::Secrets => self
                    .secrets
                    .delete(key, &self.cancel)
                    .await
                    .map_err(secret_failure),
                Step::Cache => self.delete_cache(account).await,
                Step::MediaRefs => tokio::select! {
                    () = self.cancel.cancelled() => Err(Failure::Canceled),
                    result = self.hooks.delete_media_refs(account) => result.map_err(|_|Failure::MediaRefs),
                },
                Step::StateRows => {
                    self.delete_state(account).await?;
                    Ok(())
                }
                Step::Finalize => return self.finalize(job).await,
            };
            if let Err(failure) = result {
                self.mark(job.id, job.step, Some(failure)).await?;
                return Ok(());
            }
            job.step = match job.step {
                Step::StopSession => Step::Secrets,
                Step::Secrets => Step::Cache,
                Step::Cache => Step::MediaRefs,
                Step::MediaRefs => Step::StateRows,
                Step::StateRows | Step::Finalize => Step::Finalize,
            };
            self.mark(job.id, job.step, None).await?;
        }
    }

    async fn delete_cache(&self, account: AccountId) -> Result<(), Failure> {
        let path = self.root.account_cache(account);
        let outgoing = self.root.outgoing_files(account);
        let lease = std::sync::Arc::clone(&self.registry.lease);
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            for suffix in ["", "-wal", "-shm", "-journal"] {
                let mut name = path.as_os_str().to_os_string();
                name.push(suffix);
                match std::fs::remove_file(std::path::PathBuf::from(name)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err(Failure::CacheIo),
                }
            }
            match std::fs::remove_dir_all(outgoing) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Failure::CacheIo),
            }
            Ok(())
        })
        .await
        .map_err(|_| Failure::FileTask)?
    }

    async fn delete_state(&self, account: AccountId) -> Result<(), StorageError> {
        self.registry
            .writer
            .write(move |tx| {
                for table in [
                    "operations",
                    "drafts",
                    "read_points",
                    "notification_ledger",
                    "notification_state",
                    "expression_favorites",
                    "expression_recent",
                    "expression_usage",
                ] {
                    tx.execute(
                        &format!("DELETE FROM {table} WHERE server_id=?1 AND user_id=?2"),
                        rusqlite::params![account.server().as_bytes(), account.user().as_bytes()],
                    )?;
                }
                Ok(())
            })
            .await
    }
}

fn secret_failure(error: SecretError) -> Failure {
    match error {
        SecretError::Uncertain(_) | SecretError::Failure(SecretFailure::Unfenced) => {
            Failure::SecretUncertain
        }
        SecretError::Failure(SecretFailure::Timeout) => Failure::SecretTimeout,
        SecretError::Failure(SecretFailure::Canceled) => Failure::Canceled,
        SecretError::Failure(SecretFailure::Store(_)) => Failure::SecretStore,
        SecretError::Superseded
        | SecretError::Busy
        | SecretError::UnknownAccount
        | SecretError::Poisoned => Failure::CredentialState,
    }
}
