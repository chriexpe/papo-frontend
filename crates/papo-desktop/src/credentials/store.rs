//! Synchronous core storage with a run-local memory overlay and native persistence.
//!
//! The operation lock orders memory changes and OS completions. Known missing
//! values are tombstones: a failed deletion must not reload an old native token.
//! Shutdown uses the bridge's separate short-held lock, never this waiting lock.

use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

use super::{
    CredentialError,
    bridge::{Bridge, Operation},
};
use papo_core::storage::{MemorySecretStore, Secret, SecretStore, StorageError, StorageResult};

pub(super) struct OsSecretStore {
    pub(super) bridge: Arc<Bridge>,
    memory: MemorySecretStore,
    known: Mutex<HashSet<(String, Secret)>>,
}

impl OsSecretStore {
    pub(super) fn new(bridge: Arc<Bridge>) -> Self {
        Self {
            bridge,
            memory: MemorySecretStore::default(),
            known: Mutex::default(),
        }
    }
}

impl SecretStore for OsSecretStore {
    fn load(&self, server: &str, secret: Secret) -> StorageResult<Option<String>> {
        self.bridge.check_worker().map_err(storage_error)?;
        let mut known = self
            .known
            .lock()
            .map_err(|_| storage_error(CredentialError::Internal))?;
        let key = (server.to_owned(), secret);
        if known.contains(&key) {
            return self.memory.load(server, secret);
        }
        let result = self
            .bridge
            .request(server, secret, Operation::Read)
            .map_err(storage_error)?;
        if let Some(value) = &result {
            self.memory.store(server, secret, value)?;
        }
        known.insert(key);
        Ok(result)
    }

    fn store(&self, server: &str, secret: Secret, value: &str) -> StorageResult<()> {
        self.bridge.check_worker().map_err(storage_error)?;
        let mut known = self
            .known
            .lock()
            .map_err(|_| storage_error(CredentialError::Internal))?;
        self.memory.store(server, secret, value)?;
        known.insert((server.to_owned(), secret));
        // Returning an error is intentional: a later importer must not delete
        // its source copy before the native store confirms persistence.
        self.bridge
            .request(server, secret, Operation::Write(value.to_owned()))
            .map(|_| ())
            .map_err(storage_error)
    }

    fn remove(&self, server: &str, secret: Secret) -> StorageResult<()> {
        self.bridge.check_worker().map_err(storage_error)?;
        let mut known = self
            .known
            .lock()
            .map_err(|_| storage_error(CredentialError::Internal))?;
        self.memory.remove(server, secret)?;
        known.insert((server.to_owned(), secret));
        self.bridge
            .request(server, secret, Operation::Delete)
            .map(|_| ())
            .map_err(storage_error)
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
