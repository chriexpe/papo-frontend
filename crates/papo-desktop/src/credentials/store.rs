//! Synchronous core storage with a run-local memory overlay and native persistence.
//!
//! The operation lock orders memory changes and OS completions across servers.
//! Known missing values are tombstones: a failed deletion must not reload an old
//! native token, even after rebinding. A separate generation guard covers memory
//! access only, never native waits. Revocation and shutdown do not take the
//! operation lock.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use super::{
    CredentialError,
    bridge::{Bridge, Operation},
    generation::Generation,
};
use papo_core::storage::{MemorySecretStore, Secret, SecretStore, StorageError, StorageResult};

#[derive(Default)]
pub(super) struct Overlay {
    memory: MemorySecretStore,
    known: Mutex<HashSet<(String, Secret)>>,
}

pub(super) struct OsSecretStore {
    pub(super) bridge: Arc<Bridge>,
    generation: Arc<Generation>,
    overlay: Arc<Overlay>,
}

impl OsSecretStore {
    pub(super) fn new(
        bridge: Arc<Bridge>,
        generation: Arc<Generation>,
        overlay: Arc<Overlay>,
    ) -> Self {
        Self {
            bridge,
            generation,
            overlay,
        }
    }

    fn check_worker(&self, server: &str) -> StorageResult<()> {
        self.bridge.check_worker().map_err(storage_error)?;
        drop(self.generation.lock(server).map_err(storage_error)?);
        Ok(())
    }

    fn request(
        &self,
        server: &str,
        secret: Secret,
        operation: Operation,
    ) -> StorageResult<Option<String>> {
        // Preserve native errors even if their callback initiates shutdown.
        let result = self
            .bridge
            .request(&self.generation, server, secret, operation)
            .map_err(storage_error)?;
        drop(self.generation.lock(server).map_err(storage_error)?);
        Ok(result)
    }

    /// Entered only after the initial worker/identity check; rechecks under the lock.
    fn load_admitted(&self, server: &str, secret: Secret) -> StorageResult<Option<String>> {
        let mut known = self
            .overlay
            .known
            .lock()
            .map_err(|_| storage_error(CredentialError::Internal))?;
        let key = (server.to_owned(), secret);
        {
            let _current = self.generation.lock(server).map_err(storage_error)?;
            if known.contains(&key) {
                return self.overlay.memory.load(server, secret);
            }
        }
        let result = self.request(server, secret, Operation::Read)?;
        let _current = self.generation.lock(server).map_err(storage_error)?;
        if let Some(value) = &result {
            self.overlay.memory.store(server, secret, value)?;
        }
        known.insert(key);
        Ok(result)
    }

    /// Entered only after the initial worker/identity check; rechecks under the lock.
    fn store_admitted(&self, server: &str, secret: Secret, value: &str) -> StorageResult<()> {
        let mut known = self
            .overlay
            .known
            .lock()
            .map_err(|_| storage_error(CredentialError::Internal))?;
        {
            let _current = self.generation.lock(server).map_err(storage_error)?;
            self.overlay.memory.store(server, secret, value)?;
            known.insert((server.to_owned(), secret));
        }
        // Only confirmed native persistence returns success. Revocation does not
        // undo a write already admitted or dispatched to the OS.
        self.request(server, secret, Operation::Write(value.to_owned()))
            .map(|_| ())
    }

    /// Entered only after the initial worker/identity check; rechecks under the lock.
    fn remove_admitted(&self, server: &str, secret: Secret) -> StorageResult<()> {
        let mut known = self
            .overlay
            .known
            .lock()
            .map_err(|_| storage_error(CredentialError::Internal))?;
        {
            let _current = self.generation.lock(server).map_err(storage_error)?;
            self.overlay.memory.remove(server, secret)?;
            known.insert((server.to_owned(), secret));
        }
        self.request(server, secret, Operation::Delete).map(|_| ())
    }
}

impl SecretStore for OsSecretStore {
    fn load(&self, server: &str, secret: Secret) -> StorageResult<Option<String>> {
        self.check_worker(server)?;
        self.load_admitted(server, secret)
    }

    fn store(&self, server: &str, secret: Secret, value: &str) -> StorageResult<()> {
        self.check_worker(server)?;
        self.store_admitted(server, secret, value)
    }

    fn remove(&self, server: &str, secret: Secret) -> StorageResult<()> {
        self.check_worker(server)?;
        self.remove_admitted(server, secret)
    }

    fn forget_server(&self, server: &str) -> StorageResult<()> {
        let token = self.remove(server, Secret::SessionToken);
        let password = self.remove(server, Secret::ServerPassword);
        token.and(password)
    }
}

fn storage_error(error: CredentialError) -> StorageError {
    StorageError::new(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::{OsSecretStore, Overlay};
    use crate::credentials::{Bridge, CredentialError, CredentialOperation, Generation};
    use papo_core::storage::{Secret, SecretStore as _};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn assert_revocation_after_admission(operation: CredentialOperation) {
        let (bridge, requests) = Bridge::new();
        let generation = Arc::new(Generation::new("synthetic-a", &bridge));
        let overlay = Arc::new(Overlay::default());
        overlay
            .memory
            .store("synthetic-a", Secret::SessionToken, "synthetic-existing")
            .unwrap();
        let mut held = overlay.known.lock().unwrap();
        held.insert(("synthetic-a".to_owned(), Secret::SessionToken));
        let store = OsSecretStore::new(
            Arc::clone(&bridge),
            Arc::clone(&generation),
            Arc::clone(&overlay),
        );
        let native_calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&native_calls);
        let dispatcher = std::thread::spawn(move || {
            while let Ok(request) = requests.recv_blocking() {
                observed.fetch_add(1, Ordering::Relaxed);
                request.complete(Ok(None));
            }
        });
        let (admitted, checked) = async_channel::bounded(1);
        let worker = std::thread::spawn(move || {
            // The production trait methods perform this check before entering
            // these same operation bodies. The main thread holds their lock.
            store.check_worker("synthetic-a").unwrap();
            admitted.send_blocking(()).unwrap();
            match operation {
                CredentialOperation::Read => {
                    store.load_admitted("synthetic-a", Secret::SessionToken)
                }
                CredentialOperation::Write => store
                    .store_admitted("synthetic-a", Secret::SessionToken, "synthetic-stale")
                    .map(|()| None),
                CredentialOperation::Delete => store
                    .remove_admitted("synthetic-a", Secret::SessionToken)
                    .map(|()| None),
            }
        });
        checked.recv_blocking().unwrap();
        generation.invalidate();
        drop(held);
        let result = worker.join().unwrap();
        bridge.stop();
        dispatcher.join().unwrap();
        assert_eq!(
            result.unwrap_err().to_string(),
            CredentialError::Stale.to_string()
        );
        assert_eq!(native_calls.load(Ordering::Relaxed), 0);
        assert_eq!(
            overlay
                .memory
                .load("synthetic-a", Secret::SessionToken)
                .unwrap()
                .as_deref(),
            Some("synthetic-existing")
        );
    }

    #[test]
    fn revoked_admitted_cached_read_cannot_return_old_memory() {
        assert_revocation_after_admission(CredentialOperation::Read);
    }

    #[test]
    fn revoked_admitted_write_cannot_change_memory_or_enqueue() {
        assert_revocation_after_admission(CredentialOperation::Write);
    }

    #[test]
    fn revoked_admitted_delete_cannot_change_memory_or_enqueue() {
        assert_revocation_after_admission(CredentialOperation::Delete);
    }
}
