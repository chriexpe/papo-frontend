//! Account-scoped non-message cache ownership; sessions verify identity and fence connection generations.

#[cfg(test)]
mod admission_tests;
mod encoding;
#[cfg(test)]
mod lifecycle_tests;
mod persistence;
mod schema;
#[cfg(test)]
mod tests;

use crate::{
    AccountId, DataRoot, Generation, ResourceSnapshot,
    storage_worker::{DatabaseWriter, Initializer, ReadPool, SqlBatch, WorkerError},
};
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::{Mutex, OwnedMutexGuard};

/// Sanitized cache failures; no row contents, account identity or filesystem paths are exposed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CacheError {
    /// This cache handle was closed before account removal or replacement.
    #[error("account cache is closed")]
    Closed,
    /// A newer connection generation claimed this account cache.
    #[error("account cache generation was superseded")]
    Superseded,
    /// A backend UUID or serialized cache payload is invalid.
    #[error("invalid account cache data")]
    InvalidData,
    /// A cache operation task terminated unexpectedly.
    #[error("account cache task stopped")]
    TaskStopped,
    /// The shared storage boundary refused or failed an operation.
    #[error(transparent)]
    Storage(#[from] WorkerError),
}

/// Persistable resource groups. Presence, typing and activity have no cache variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CacheResource {
    /// Server metadata, excluding image bytes.
    Server,
    /// Channels and their permission overwrites.
    Channels,
    /// Role definitions.
    Roles,
    /// Membership summaries and their role memberships.
    Members,
    /// On-demand profiles, excluding image bytes and live state.
    Profiles,
    /// Direct conversations, with live fields removed from nested summaries.
    Dms,
    /// Blocked-user summaries.
    Blocks,
}

const ALL_RESOURCES: [CacheResource; 7] = [
    CacheResource::Server,
    CacheResource::Channels,
    CacheResource::Roles,
    CacheResource::Members,
    CacheResource::Profiles,
    CacheResource::Dms,
    CacheResource::Blocks,
];
pub(crate) type Registrar =
    Arc<dyn Fn(PathBuf, Initializer) -> Result<DatabaseWriter, WorkerError> + Send + Sync>;
type Fence = OwnedMutexGuard<bool>;

/// Cloneable lazy cache factory sharing Storage's NORMAL writer and bounded read pool.
/// Construction opens no account files and does not create a thread.
#[derive(Clone)]
pub struct AccountCaches {
    inner: Arc<CacheFactory>,
}

struct CacheFactory {
    root: DataRoot,
    registrar: Registrar,
    readers: Arc<ReadPool>,
    accounts: Mutex<HashMap<AccountId, Arc<CacheSlot>>>,
}

struct CacheSlot {
    cache: Arc<Mutex<Option<Arc<AccountCache>>>>,
}

impl CacheFactory {
    async fn slot(&self, account: AccountId) -> Arc<CacheSlot> {
        let mut accounts = self.accounts.lock().await;
        Arc::clone(accounts.entry(account).or_insert_with(|| {
            Arc::new(CacheSlot {
                cache: Arc::new(Mutex::new(None)),
            })
        }))
    }
}

/// One verified account's rebuildable resource cache. All clones share one operation fence.
/// Session validity belongs to callers; this cache only rejects superseded generation claims.
pub struct AccountCache {
    account: AccountId,
    path: PathBuf,
    writer: DatabaseWriter,
    readers: Arc<ReadPool>,
    serial: Arc<Mutex<bool>>,
    generation: Arc<AtomicU64>,
}

impl AccountCaches {
    pub(crate) fn new(root: DataRoot, registrar: Registrar, readers: Arc<ReadPool>) -> Self {
        Self {
            inner: Arc::new(CacheFactory {
                root,
                registrar,
                readers,
                accounts: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Open only after the caller verified `whoami` matches this account. Repeated opens
    /// share a handle. Schema/created-for checks complete before any resource read.
    /// Cancellation before lifecycle admission starts no detached work. Once admitted,
    /// cancellation retains the close fence until the registered writer is fully installed.
    ///
    /// # Errors
    /// Returns sanitized filesystem, initialization, registration or storage failures.
    pub async fn open(&self, account: AccountId) -> Result<Arc<AccountCache>, CacheError> {
        let inner = Arc::clone(&self.inner);
        let slot = inner.slot(account).await;
        let mut registered = Arc::clone(&slot.cache).lock_owned().await;
        tokio::spawn(async move {
            if let Some(cache) = registered.as_ref() {
                return Ok(Arc::clone(cache));
            }
            let path = inner.root.account_cache(account);
            let parent = path.parent().ok_or(CacheError::InvalidData)?.to_path_buf();
            tokio::task::spawn_blocking(move || std::fs::create_dir_all(parent))
                .await
                .map_err(|_| CacheError::TaskStopped)?
                .map_err(|_| WorkerError::System)?;
            let writer = (inner.registrar)(
                path.clone(),
                Arc::new(move |connection| schema::initialize(connection, account)),
            )?;
            if let Err(error) = writer.commit(SqlBatch::new(Vec::new())?).await {
                let _ = writer.close().await;
                return Err(error.into());
            }
            let cache = Arc::new(AccountCache {
                account,
                path,
                writer,
                readers: Arc::clone(&inner.readers),
                serial: Arc::new(Mutex::new(false)),
                generation: Arc::new(AtomicU64::new(0)),
            });
            *registered = Some(Arc::clone(&cache));
            Ok(cache)
        })
        .await
        .map_err(|_| CacheError::TaskStopped)?
    }

    /// Fence actual blocking reads and accepted writes, then drain/checkpoint/close the
    /// account writer before files can be removed. Idempotent, including canceled callers.
    /// A later verified open creates a fresh handle; old handles remain closed.
    ///
    /// # Errors
    /// Returns the writer's sanitized close/checkpoint error after its connection is dropped.
    pub async fn close(&self, account: AccountId) -> Result<(), CacheError> {
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let slot = inner.slot(account).await;
            let mut registered = slot.cache.lock().await;
            let Some(cache) = registered.as_ref() else {
                return Ok(());
            };
            let mut closed = Arc::clone(&cache.serial).lock_owned().await;
            *closed = true;
            let result = cache.writer.close().await.map_err(CacheError::from);
            registered.take();
            result
        })
        .await
        .map_err(|_| CacheError::TaskStopped)?
    }

    /// Close a failed cache and discard its database and exact SQLite sidecars. The
    /// factory excludes concurrent reopen until deletion finishes; other accounts survive.
    /// Use only after a cache error, never for ordinary disconnects or superseded generations.
    ///
    /// # Errors
    /// Returns sanitized deletion or close errors. A close error is reported after discard.
    pub async fn invalidate(&self, account: AccountId) -> Result<(), CacheError> {
        let inner = Arc::clone(&self.inner);
        tokio::spawn(async move {
            let slot = inner.slot(account).await;
            let mut registered = slot.cache.lock().await;
            let mut close_result = Ok(());
            if let Some(cache) = registered.as_ref() {
                let mut closed = Arc::clone(&cache.serial).lock_owned().await;
                *closed = true;
                close_result = cache.writer.close().await.map_err(CacheError::from);
                registered.take();
            }
            let path = inner.root.account_cache(account);
            tokio::task::spawn_blocking(move || {
                for suffix in ["-wal", "-shm", ""] {
                    let mut target = path.as_os_str().to_os_string();
                    target.push(suffix);
                    match std::fs::remove_file(target) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(_) => return Err(CacheError::Storage(WorkerError::System)),
                    }
                }
                Ok(())
            })
            .await
            .map_err(|_| CacheError::TaskStopped)??;
            close_result
        })
        .await
        .map_err(|_| CacheError::TaskStopped)?
    }
}

impl AccountCache {
    /// Load the visible coherent revision, tagged with the caller's current generation.
    /// Identity metadata is checked before every bounded query batch. Presence is empty.
    /// Canceling this future does not release the fence around a running blocking query.
    ///
    /// # Errors
    /// Returns closed, invalid-payload or sanitized read-pool failures.
    pub async fn load(&self, generation: Generation) -> Result<ResourceSnapshot, CacheError> {
        self.claim(generation)?;
        let fence = self.fence().await?;
        let context = self.context();
        tokio::spawn(async move { persistence::load(&context, fence, generation).await })
            .await
            .map_err(|_| CacheError::TaskStopped)?
    }

    /// Replace all persisted resources in one visible revision switch. Staging and old-row
    /// cleanup use at most 500 affected rows per shared-writer turn. Live state is omitted.
    ///
    /// # Errors
    /// Returns closed, invalid-backend-data or sanitized write failures.
    pub async fn replace(&self, snapshot: &ResourceSnapshot) -> Result<(), CacheError> {
        self.update(snapshot, &ALL_RESOURCES).await
    }

    /// Persist only selected resource groups, atomically. Unselected groups are neither
    /// cloned, encoded nor rewritten; an empty selection performs no I/O. Callers must
    /// omit presence-only updates and validate connection generations before admission.
    /// Once accepted, cancellation of the caller cannot interrupt staging or release its fence.
    ///
    /// # Errors
    /// Returns closed, invalid-backend-data or sanitized write/checkpoint failures.
    pub async fn update(
        &self,
        snapshot: &ResourceSnapshot,
        changed: &[CacheResource],
    ) -> Result<(), CacheError> {
        let generation = snapshot.generation;
        self.claim(generation)?;
        let fence = self.fence().await?;
        if changed.is_empty() {
            return Ok(());
        }
        let selected = encoding::select(snapshot, changed);
        let context = self.context();
        tokio::spawn(async move {
            context.validate(generation)?;
            let encoded = tokio::task::spawn_blocking(move || encoding::encode(selected))
                .await
                .map_err(|_| CacheError::TaskStopped)??;
            persistence::replace(&context, fence, encoded, generation).await
        })
        .await
        .map_err(|_| CacheError::TaskStopped)?
    }

    async fn fence(&self) -> Result<Fence, CacheError> {
        let fence = Arc::clone(&self.serial).lock_owned().await;
        if *fence {
            return Err(CacheError::Closed);
        }
        Ok(fence)
    }

    fn claim(&self, generation: Generation) -> Result<(), CacheError> {
        let previous = self
            .generation
            .fetch_max(generation.get(), Ordering::AcqRel);
        if previous > generation.get() {
            return Err(CacheError::Superseded);
        }
        Ok(())
    }

    fn context(&self) -> CacheContext {
        CacheContext {
            account: self.account,
            path: self.path.clone(),
            writer: self.writer.clone(),
            readers: Arc::clone(&self.readers),
            generation: Arc::clone(&self.generation),
        }
    }
}

struct CacheContext {
    account: AccountId,
    path: PathBuf,
    writer: DatabaseWriter,
    readers: Arc<ReadPool>,
    generation: Arc<AtomicU64>,
}

impl CacheContext {
    fn validate(&self, generation: Generation) -> Result<(), CacheError> {
        if self.generation.load(Ordering::Acquire) != generation.get() {
            return Err(CacheError::Superseded);
        }
        Ok(())
    }

    async fn query<T, F>(&self, fence: Fence, query: F) -> Result<(Fence, T), CacheError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Transaction<'_>) -> Result<T, CacheError> + Send + 'static,
    {
        let account = self.account;
        let (fence, result) = self
            .readers
            .query(self.path.clone(), move |transaction| {
                // Returning the guard keeps it alive through transaction commit and connection drop,
                // even if the awaiting task or its caller disappears while spawn_blocking runs.
                let result = schema::metadata(transaction, account)
                    .map_err(WorkerError::from)
                    .map_err(CacheError::from)
                    .and_then(|_| query(transaction));
                Ok((fence, result))
            })
            .await?;
        Ok((fence, result?))
    }
}
