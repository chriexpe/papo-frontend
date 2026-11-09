//! Scripted in-memory credential host for deterministic SDK tests; never an OS implementation.

use std::{
    collections::{HashMap, VecDeque},
    future::pending,
    sync::{Arc, Mutex},
    time::Duration,
};

use super::{SecretFuture, SecretKey, SecretRead, SecretStore, SecretStoreError, SecretValue};

/// Platform operation selected by a script.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Operation {
    /// Read an entry.
    Read,
    /// Replace an entry.
    Write,
    /// Remove an entry.
    Delete,
}

/// Behavior of the next operation of a selected kind.
#[derive(Clone, Copy, Debug)]
pub enum Behavior {
    /// Complete normally on first poll.
    Complete,
    /// Never finish; dropping the future prevents any effects.
    Stall,
    /// Finish normally after a Tokio-clock delay, unless the future is dropped.
    Delay(Duration),
    /// Apply effects on a detached task after a Tokio-clock delay; never return.
    Detached(Duration),
    /// Return a typed error without effects.
    Fail(SecretStoreError),
}

/// In-memory test capability with per-operation scripts and mutation fencing.
#[derive(Clone, Default)]
pub struct ScriptedSecretStore {
    state: Arc<Mutex<State>>,
}

/// Test-only state; intentionally not formattable because it contains credentials.
#[derive(Default)]
struct State {
    entries: HashMap<SecretKey, Vec<u8>>,
    scripts: HashMap<Operation, VecDeque<Behavior>>,
    calls: HashMap<Operation, usize>,
    pending: HashMap<SecretKey, usize>,
}

impl ScriptedSecretStore {
    /// Appends behavior for one operation kind; unspecified calls complete normally.
    ///
    /// # Panics
    /// Panics if another test action poisoned the in-memory state mutex.
    pub fn push(&self, operation: Operation, behavior: Behavior) {
        self.state
            .lock()
            .unwrap()
            .scripts
            .entry(operation)
            .or_default()
            .push_back(behavior);
    }

    /// Counts calls that were polled, not merely constructed.
    ///
    /// # Panics
    /// Panics if another test action poisoned the in-memory state mutex.
    #[must_use]
    pub fn calls(&self, operation: Operation) -> usize {
        self.state
            .lock()
            .unwrap()
            .calls
            .get(&operation)
            .copied()
            .unwrap_or_default()
    }

    /// Inspects physical bytes without going through SDK reconciliation.
    ///
    /// # Panics
    /// Panics if another test action poisoned the in-memory state mutex.
    #[must_use]
    pub fn snapshot(&self, key: SecretKey) -> Option<SecretValue> {
        self.state
            .lock()
            .unwrap()
            .entries
            .get(&key)
            .cloned()
            .map(SecretValue::new)
    }

    /// Runs a script only when polled, matching the platform future contract.
    async fn execute(
        &self,
        key: SecretKey,
        operation: Operation,
        value: Option<SecretValue>,
    ) -> Result<SecretRead, SecretStoreError> {
        let behavior = {
            let mut state = self.state.lock().unwrap();
            *state.calls.entry(operation).or_default() += 1;
            state
                .scripts
                .entry(operation)
                .or_default()
                .pop_front()
                .unwrap_or(Behavior::Complete)
        };
        match behavior {
            Behavior::Complete => {}
            Behavior::Stall => return pending().await,
            Behavior::Fail(error) => return Err(error),
            Behavior::Delay(duration) => tokio::time::sleep(duration).await,
            Behavior::Detached(duration) => {
                let state = Arc::clone(&self.state);
                if operation != Operation::Read {
                    *state.lock().unwrap().pending.entry(key).or_default() += 1;
                }
                tokio::spawn(async move {
                    tokio::time::sleep(duration).await;
                    let mut state = state.lock().unwrap();
                    Self::apply(&mut state, key, operation, value);
                    if operation != Operation::Read {
                        *state.pending.entry(key).or_default() -= 1;
                    }
                });
                return pending().await;
            }
        }
        let mut state = self.state.lock().unwrap();
        Self::apply(&mut state, key, operation, value);
        let value = state.entries.get(&key).cloned().map(SecretValue::new);
        Ok(
            if state.pending.get(&key).copied().unwrap_or_default() == 0 {
                SecretRead::settled(value)
            } else {
                SecretRead::unfenced(value)
            },
        )
    }

    /// Applies effects atomically with the test host's mutation fence accounting.
    fn apply(state: &mut State, key: SecretKey, operation: Operation, value: Option<SecretValue>) {
        match operation {
            Operation::Read => {}
            Operation::Write => {
                state.entries.insert(key, value.unwrap().expose().to_vec());
            }
            Operation::Delete => {
                state.entries.remove(&key);
            }
        }
    }
}

impl SecretStore for ScriptedSecretStore {
    fn read(&self, key: SecretKey) -> SecretFuture<'_, SecretRead> {
        Box::pin(self.execute(key, Operation::Read, None))
    }

    fn write(&self, key: SecretKey, value: SecretValue) -> SecretFuture<'_, ()> {
        Box::pin(async move {
            self.execute(key, Operation::Write, Some(value))
                .await
                .map(|_| ())
        })
    }

    fn delete(&self, key: SecretKey) -> SecretFuture<'_, ()> {
        Box::pin(async move { self.execute(key, Operation::Delete, None).await.map(|_| ()) })
    }
}
