//! Revocable server incarnations shared by requests, stores and deferred notices.
//!
//! The short-held guard makes revocation atomic with memory-overlay changes.
//! It never covers native I/O, waits or callbacks, so revocation cannot wait for
//! an OS prompt. A generation is never reactivated, even for the same account.

use std::sync::{Arc, Mutex, MutexGuard, Weak};

use super::{CredentialError, bridge::Bridge};

pub(super) struct Generation {
    server: String,
    current: Mutex<bool>,
    bridge: Weak<Bridge>,
}

impl Generation {
    pub(super) fn new(server: &str, bridge: &Arc<Bridge>) -> Self {
        Self {
            server: server.to_owned(),
            current: Mutex::new(true),
            bridge: Arc::downgrade(bridge),
        }
    }

    pub(super) fn is_current(&self) -> bool {
        self.lock(&self.server).is_ok()
    }

    /// Hold only across run-local memory access, never a native request or wait.
    pub(super) fn lock(&self, server: &str) -> Result<MutexGuard<'_, bool>, CredentialError> {
        if server != self.server {
            return Err(CredentialError::WrongServer);
        }
        let current = self.current.lock().map_err(|_| CredentialError::Internal)?;
        if !*current {
            return Err(CredentialError::Stale);
        }
        if !self
            .bridge
            .upgrade()
            .is_some_and(|bridge| bridge.is_running())
        {
            return Err(CredentialError::Stopped);
        }
        Ok(current)
    }

    pub(super) fn invalidate(&self) {
        *self
            .current
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
    }
}
