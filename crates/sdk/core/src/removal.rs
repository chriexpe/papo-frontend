//! Recoverable cross-store removal; sessions and media own the supplied cleanup hooks.

use crate::secret::{SecretKey, SecretStoreBoundary};
use crate::{AccountId, DataRoot, Registry, ServerId, StorageError};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio_util::sync::CancellationToken;

mod journal;
mod steps;

/// A removal target; a server job follows all of its account jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RemovalTarget {
    /// Remove one account without removing the server registry entry.
    Account(AccountId),
    /// Remove the server after all account cleanup has completed.
    Server(ServerId),
}

/// The next idempotent step to execute, persisted after each successful step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i64)]
#[non_exhaustive]
pub enum RemovalStep {
    /// Revoke session handles and close any cache connections through the hook.
    StopSession = 0,
    /// Delete generation-qualified OS credentials through the boundary.
    Secrets = 1,
    /// Delete the account cache and SQLite sidecars.
    Cache = 2,
    /// Delete account media references through the hook.
    MediaRefs = 3,
    /// Delete account-owned durable state.
    StateRows = 4,
    /// Atomically delete account/server metadata and the completed job.
    Finalize = 5,
}

/// Stable typed failure codes persisted without platform messages or secret data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i64)]
#[non_exhaustive]
pub enum RemovalFailureKind {
    /// Session cleanup hook failed.
    SessionStop = 1,
    /// A credential read timed out.
    SecretTimeout = 2,
    /// Cancellation interrupted a step.
    Canceled = 3,
    /// The credential store reported a definite failure.
    SecretStore = 4,
    /// Credential mutation outcome cannot yet be established by fenced read-back.
    SecretUncertain = 5,
    /// The credential boundary rejected the generation or concurrent use.
    CredentialState = 6,
    /// Removing the cache or a sidecar failed.
    CacheIo = 7,
    /// Media cleanup hook failed.
    MediaRefs = 8,
    /// A blocking filesystem task failed to complete normally.
    FileTask = 9,
}

/// Persisted removal progress, including the last typed failure if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovalJob {
    /// Durable monotonically allocated job identity.
    pub id: i64,
    /// Hidden identity being removed.
    pub target: RemovalTarget,
    /// Next unacknowledged cleanup step.
    pub step: RemovalStep,
    /// Last failure, cleared only after a successful retry.
    pub failure: Option<RemovalFailureKind>,
    pub(crate) key: Option<SecretKey>,
}

/// Failure returned by a cleanup hook, without platform-provided strings.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RemovalHookError {
    /// The owner could not complete cleanup; the removal remains pending.
    #[error("removal cleanup incomplete")]
    Incomplete,
}

/// A nonblocking cleanup operation implemented by the capability that owns it.
pub type RemovalFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), RemovalHookError>> + Send + 'a>>;

/// Integration boundary only: session and media implementations are not part of storage.
/// Both methods must be idempotent. Session stop must revoke handles, stop new
/// credential admission, cancel in-flight credential users, and close cache connections
/// before resolving. No later mutation may recreate this generation's secrets or cache.
/// Media cleanup must remove only this account's refs.
pub trait RemovalHooks: Send + Sync + 'static {
    /// Revokes the account session and waits for admitted credential users to leave.
    fn stop_session(&self, account: AccountId) -> RemovalFuture<'_>;
    /// Removes media refs; there is no media schema or implementation yet.
    fn delete_media_refs(&self, account: AccountId) -> RemovalFuture<'_>;
}

/// Serialized recoverable removal runner. Failures remain available in `pending`.
#[derive(Clone)]
pub struct RemovalJobs {
    pub(crate) registry: Registry,
    pub(crate) root: DataRoot,
    pub(crate) secrets: Arc<SecretStoreBoundary>,
    pub(crate) hooks: Arc<dyn RemovalHooks>,
    pub(crate) cancel: CancellationToken,
    pub(crate) serial: Arc<tokio::sync::Mutex<()>>,
}

impl RemovalJobs {
    /// Commits a removal job and hides the account atomically; cleanup is resumed separately.
    ///
    /// # Errors
    /// Returns `NotFound` for an unavailable account or a storage commit failure.
    pub async fn begin_account(&self, account: AccountId) -> Result<(), StorageError> {
        self.registry
            .writer
            .write(move |tx| {
                crate::registry::visible_account(tx, account)?;
                tx.execute(
                    "UPDATE accounts SET hidden=1,active=0 WHERE server_id=?1 AND user_id=?2",
                    rusqlite::params![account.server().as_bytes(), account.user().as_bytes()],
                )?;
                tx.execute(
                    "INSERT INTO removal_jobs(server_id,user_id) VALUES(?1,?2)",
                    rusqlite::params![account.server().as_bytes(), account.user().as_bytes()],
                )?;
                Ok(())
            })
            .await
    }

    /// Commits account jobs, a final server job, and list hiding in one FULL transaction.
    ///
    /// # Errors
    /// Returns `NotFound` for a hidden/missing server, or a storage failure.
    pub async fn begin_server(&self, server: ServerId) -> Result<(), StorageError> {
        self.registry.writer.write(move |tx| {
            crate::registry::visible_server(tx,server)?;
            tx.execute("UPDATE servers SET hidden=1 WHERE id=?",[server.as_bytes()])?;
            tx.execute("UPDATE accounts SET hidden=1,active=0 WHERE server_id=?",[server.as_bytes()])?;
            tx.execute("INSERT OR IGNORE INTO removal_jobs(server_id,user_id) SELECT server_id,user_id FROM accounts WHERE server_id=?",[server.as_bytes()])?;
            tx.execute("INSERT INTO removal_jobs(server_id,user_id) VALUES(?,NULL)",[server.as_bytes()])?;
            Ok(())
        }).await
    }

    /// Retries every pending job once, account jobs before server finalizers.
    /// Jobs with external failures stay hidden, with a persistent typed error.
    ///
    /// # Errors
    /// Returns a durable database failure; successful external steps may safely repeat.
    pub async fn resume(&self) -> Result<Vec<RemovalJob>, StorageError> {
        let _guard = self.serial.lock().await;
        for job in self.pending().await? {
            self.run(job).await?;
        }
        self.pending().await
    }
}
