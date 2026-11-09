//! SDK credential admission and reconciliation; hosts own execution and durable generation allocation.

use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio_util::sync::CancellationToken;

use super::{
    CredentialGeneration, CredentialState, SecretError, SecretFailure, SecretKey, SecretRead,
    SecretStore, SecretStoreError, SecretValue,
};
use crate::AccountId;

/// Bounded credential access with independent state for each account.
///
/// Keep one shared instance for all users of a platform store. Admission is bounded
/// to one call per active account generation; concurrent callers receive `Busy`.
/// No state lock is held while calling the host or awaiting a future. Replacing a
/// generation does not wait for old OS work and cannot undo its physical effects.
pub struct SecretStoreBoundary {
    store: Arc<dyn SecretStore>,
    timeout: Duration,
    accounts: Mutex<HashMap<AccountId, Entry>>,
}

/// Admission state for an active generation; contains no credential bytes.
struct Entry {
    generation: CredentialGeneration,
    state: CredentialState,
    busy: bool,
}

impl SecretStoreBoundary {
    /// Sets the deadline applied independently to every platform read/write/delete.
    #[must_use]
    pub fn new(store: Arc<dyn SecretStore>, timeout: Duration) -> Self {
        Self {
            store,
            timeout,
            accounts: Mutex::new(HashMap::new()),
        }
    }

    /// Activates a durably allocated generation, invalidating older completions.
    ///
    /// New activations start uncertain: a newly created boundary cannot infer that
    /// an earlier process left no OS work outstanding. Repeating the current
    /// generation preserves its state and admission; older generations are rejected.
    ///
    /// # Errors
    /// Returns `Superseded` for a decreasing generation or `Poisoned` on lock failure.
    pub fn activate(
        &self,
        account: AccountId,
        generation: CredentialGeneration,
    ) -> Result<(), SecretError> {
        let mut accounts = self.accounts.lock().map_err(|_| SecretError::Poisoned)?;
        if let Some(entry) = accounts.get(&account) {
            if generation < entry.generation {
                return Err(SecretError::Superseded);
            }
            if generation == entry.generation {
                return Ok(());
            }
        }
        accounts.insert(
            account,
            Entry {
                generation,
                state: CredentialState::Uncertain,
                busy: false,
            },
        );
        Ok(())
    }

    /// Returns reconciliation state, without exposing credential bytes.
    ///
    /// # Errors
    /// Returns `UnknownAccount`, `Superseded`, or `Poisoned` for invalid access.
    pub fn state(&self, key: SecretKey) -> Result<CredentialState, SecretError> {
        let accounts = self.accounts.lock().map_err(|_| SecretError::Poisoned)?;
        let entry = accounts
            .get(&key.account())
            .ok_or(SecretError::UnknownAccount)?;
        if entry.generation != key.generation() {
            return Err(SecretError::Superseded);
        }
        Ok(entry.state)
    }

    /// Reads credentials, requiring a mutation fence when the key is uncertain.
    ///
    /// # Errors
    /// Returns typed admission, platform, timeout, cancellation, or uncertainty errors.
    /// An unfenced observation of an uncertain key is never returned to the caller.
    pub async fn read(
        &self,
        key: SecretKey,
        cancel: &CancellationToken,
    ) -> Result<Option<SecretValue>, SecretError> {
        let call = self.begin(key)?;
        call.read_back(cancel).await
    }

    /// Replaces credentials after mandatory fenced read-back of any uncertainty.
    ///
    /// # Errors
    /// Returns admission or reconciliation failures. Once mutation is admitted,
    /// timeout, cancellation, and platform failure all return `Uncertain`.
    /// Dropping this future during mutation also leaves the key uncertain.
    pub async fn write(
        &self,
        key: SecretKey,
        value: SecretValue,
        cancel: &CancellationToken,
    ) -> Result<(), SecretError> {
        let call = self.begin(key)?;
        if self.state(key)? == CredentialState::Uncertain {
            call.read_back(cancel).await?;
        }
        call.mutate(self.store.write(key, value), cancel).await
    }

    /// Idempotently deletes credentials, reconciling any earlier unknown mutation.
    ///
    /// Fenced read-back proving absence completes an uncertain deletion without
    /// another mutation. An unfenced absence must never complete a removal job.
    ///
    /// # Errors
    /// Returns admission/reconciliation failures or `Uncertain` if deletion has
    /// unknown effects. A superseded completion cannot acknowledge current removal.
    pub async fn delete(
        &self,
        key: SecretKey,
        cancel: &CancellationToken,
    ) -> Result<(), SecretError> {
        let call = self.begin(key)?;
        if self.state(key)? == CredentialState::Uncertain && call.read_back(cancel).await?.is_none()
        {
            return Ok(());
        }
        call.mutate(self.store.delete(key), cancel).await
    }

    /// Admits one operation without waiting on another credential request.
    fn begin(&self, key: SecretKey) -> Result<Call<'_>, SecretError> {
        let mut accounts = self.accounts.lock().map_err(|_| SecretError::Poisoned)?;
        let entry = accounts
            .get_mut(&key.account())
            .ok_or(SecretError::UnknownAccount)?;
        if entry.generation != key.generation() {
            return Err(SecretError::Superseded);
        }
        if entry.busy {
            return Err(SecretError::Busy);
        }
        entry.busy = true;
        Ok(Call {
            boundary: self,
            key,
        })
    }

    /// Races each host call against an SDK deadline and caller cancellation.
    async fn bounded<T>(
        &self,
        future: impl Future<Output = Result<T, SecretStoreError>>,
        cancel: &CancellationToken,
    ) -> Result<T, SecretFailure> {
        tokio::select! {
            biased;
            () = cancel.cancelled() => Err(SecretFailure::Canceled),
            result = tokio::time::timeout(self.timeout, future) => match result {
                Ok(result) => result.map_err(SecretFailure::Store),
                Err(_) => Err(SecretFailure::Timeout),
            },
        }
    }
}

/// Releases admission on any exit, including task abort; mutation state is set before await.
struct Call<'a> {
    boundary: &'a SecretStoreBoundary,
    key: SecretKey,
}

impl Call<'_> {
    /// Updates state only if the result still belongs to the active generation.
    fn mark(&self, state: CredentialState) -> Result<(), SecretError> {
        let mut accounts = self
            .boundary
            .accounts
            .lock()
            .map_err(|_| SecretError::Poisoned)?;
        let entry = accounts
            .get_mut(&self.key.account())
            .ok_or(SecretError::UnknownAccount)?;
        if entry.generation != self.key.generation() {
            return Err(SecretError::Superseded);
        }
        entry.state = state;
        Ok(())
    }

    /// Rejects stale results before exposing their value or interpreting their failure.
    async fn read_back(
        &self,
        cancel: &CancellationToken,
    ) -> Result<Option<SecretValue>, SecretError> {
        let uncertain = self.boundary.state(self.key)? == CredentialState::Uncertain;
        let result = self
            .boundary
            .bounded(self.boundary.store.read(self.key), cancel)
            .await;
        self.boundary.state(self.key)?;
        let SecretRead { value, settled } = result.map_err(|failure| {
            if uncertain {
                SecretError::Uncertain(failure)
            } else {
                SecretError::Failure(failure)
            }
        })?;
        if uncertain && !settled {
            return Err(SecretError::Uncertain(SecretFailure::Unfenced));
        }
        self.mark(CredentialState::Ready)?;
        Ok(value)
    }

    /// Conservatively marks in-flight mutation before polling host work.
    async fn mutate(
        &self,
        future: impl Future<Output = Result<(), SecretStoreError>>,
        cancel: &CancellationToken,
    ) -> Result<(), SecretError> {
        self.mark(CredentialState::Uncertain)?;
        let result = self.boundary.bounded(future, cancel).await;
        self.boundary.state(self.key)?;
        result.map_err(SecretError::Uncertain)?;
        self.mark(CredentialState::Ready)
    }
}

impl Drop for Call<'_> {
    fn drop(&mut self) {
        if let Ok(mut accounts) = self.boundary.accounts.lock()
            && let Some(entry) = accounts.get_mut(&self.key.account())
            && entry.generation == self.key.generation()
        {
            entry.busy = false;
        }
    }
}
