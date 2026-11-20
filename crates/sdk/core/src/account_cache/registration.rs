//! Opens account-scoped cache registrations. Only full reconciliation may renew a failed writer.

use super::{AccountCache, AccountCaches, CacheError, history, schema};
use crate::{
    AccountId,
    storage_worker::{SqlBatch, WorkerError},
};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicU64},
};
use tokio::sync::Mutex;

impl AccountCaches {
    /// Open only after the caller verified `whoami` matches this account. Repeated opens
    /// share a handle. Schema/created-for checks complete before any resource read.
    /// Cancellation before lifecycle admission starts no detached work. Once admitted,
    /// cancellation retains the close fence until the registered writer is fully installed.
    ///
    /// # Errors
    /// Returns sanitized filesystem, initialization, registration or storage failures.
    pub async fn open(&self, account: AccountId) -> Result<Arc<AccountCache>, CacheError> {
        self.open_registered(account, false).await
    }

    /// Full session reconciliation can replace a failed writer while preserving its
    /// committed revision and history. Partial producers keep their original fence.
    pub(crate) async fn for_reconcile(
        &self,
        account: AccountId,
    ) -> Result<Arc<AccountCache>, CacheError> {
        self.open_registered(account, true).await
    }

    async fn open_registered(
        &self,
        account: AccountId,
        replace_failed: bool,
    ) -> Result<Arc<AccountCache>, CacheError> {
        let inner = Arc::clone(&self.inner);
        let slot = inner.slot(account).await;
        let mut registered = Arc::clone(&slot.cache).lock_owned().await;
        tokio::spawn(async move {
            if let Some(cache) = registered.as_ref() {
                if !replace_failed || !cache.writer.failed() {
                    return Ok(Arc::clone(cache));
                }
                let mut closed = cache.serial.lock().await;
                *closed = true;
                // close returns the original terminal fault after draining. Its completion
                // fences SQLite ownership so the same file can now be safely registered again.
                let _ = cache.writer.close().await;
            }
            registered.take();
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
                retention: std::sync::Mutex::new(HashMap::new()),
                revokes: history::RevokeQueue::default(),
            });
            if let Err(error) = cache.history_recover().await {
                let _ = cache.writer.close().await;
                return Err(error);
            }
            *registered = Some(Arc::clone(&cache));
            Ok(cache)
        })
        .await
        .map_err(|_| CacheError::TaskStopped)?
    }
}
