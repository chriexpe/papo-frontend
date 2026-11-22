//! Dedicated OS-thread bridge between `papo_sdk::secret::SecretStore` and Kotlin `FfiSecretStore`.
//! Polling a `SecretFuture` never blocks Tokio, work starts lazily on first poll, abandoned requests
//! skip OS execution, and `settled = false` is preserved verbatim so unfenced reads keep keys `Uncertain`.

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use papo_sdk::secret::{
    SecretFuture, SecretKey, SecretRead, SecretStore, SecretStoreError, SecretValue,
};
use tokio::sync::oneshot;

use crate::ids::FfiAccountId;

/// Generation-qualified OS credential key passed to Android `SecretStore`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiSecretKey {
    /// Owning account identifier (server + user).
    pub account: FfiAccountId,
    /// Monotonic, never-reused durable credential epoch.
    pub generation: u64,
}

impl From<SecretKey> for FfiSecretKey {
    fn from(key: SecretKey) -> Self {
        Self {
            account: FfiAccountId::from_sdk(key.account()),
            generation: key.generation().get(),
        }
    }
}

/// Credential read outcome including platform quiescence proof (`settled`).
/// Deliberately omits `Debug` / logging helpers so bytes never enter diagnostics.
#[derive(uniffi::Record)]
pub struct FfiSecretRead {
    /// Credential bytes if present in the platform store; `None` when absent.
    pub value: Option<Vec<u8>>,
    /// True ONLY when every earlier mutation for this exact key (including timed-out/canceled writes
    /// or writes on earlier store instances in this process) has completed before this read.
    pub settled: bool,
}

/// Platform credential store error categories without OS diagnostics or secret bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiSecretStoreError {
    /// Platform keystore is temporarily unavailable.
    #[error("credential store unavailable")]
    Unavailable,
    /// Platform rejected access (e.g., locked keystore or permanent key invalidation).
    #[error("credential store access denied")]
    AccessDenied,
    /// Operation failed; write/delete side effects may still have landed.
    #[error("credential store operation failed")]
    Failed,
}

impl From<FfiSecretStoreError> for SecretStoreError {
    fn from(error: FfiSecretStoreError) -> Self {
        match error {
            FfiSecretStoreError::Unavailable => Self::Unavailable,
            FfiSecretStoreError::AccessDenied => Self::AccessDenied,
            FfiSecretStoreError::Failed => Self::Failed,
        }
    }
}

impl From<uniffi::UnexpectedUniFFICallbackError> for FfiSecretStoreError {
    fn from(_: uniffi::UnexpectedUniFFICallbackError) -> Self {
        Self::Failed
    }
}

/// Foreign credential store implemented by Android (`AndroidSecretStore`) or host tests.
///
/// Implementations must execute mutations serially on a process-wide keystore gate or return
/// `settled = false` whenever quiescence of older work cannot be proven.
#[uniffi::export(with_foreign)]
pub trait FfiSecretStore: Send + Sync {
    /// Reads the exact generation-qualified entry; absence returns `Ok(FfiSecretRead { value: None, .. })`.
    ///
    /// # Errors
    /// Returns [`FfiSecretStoreError`] without formatting credential bytes.
    fn read(&self, key: FfiSecretKey) -> Result<FfiSecretRead, FfiSecretStoreError>;
    /// Replaces the entry for `key`; returning `Ok(())` proves OS durability ended.
    ///
    /// # Errors
    /// Returns [`FfiSecretStoreError`]; failures keep effects uncertain until a fenced read completes.
    fn write(&self, key: FfiSecretKey, value: Vec<u8>) -> Result<(), FfiSecretStoreError>;
    /// Idempotently removes only `key`, including when absent.
    ///
    /// # Errors
    /// Returns [`FfiSecretStoreError`]; failures keep effects uncertain until a fenced read completes.
    fn delete(&self, key: FfiSecretKey) -> Result<(), FfiSecretStoreError>;
}

/// One queued operation dispatched onto the dedicated `"papo-ffi-secret"` OS thread.
enum SecretJob {
    /// Read entry and report both value and `settled` fence.
    Read {
        key: SecretKey,
        abandoned: Arc<AtomicBool>,
        reply: oneshot::Sender<Result<SecretRead, SecretStoreError>>,
    },
    /// Write credential bytes under exact key.
    Write {
        key: SecretKey,
        value: SecretValue,
        abandoned: Arc<AtomicBool>,
        reply: oneshot::Sender<Result<(), SecretStoreError>>,
    },
    /// Delete entry for exact key.
    Delete {
        key: SecretKey,
        abandoned: Arc<AtomicBool>,
        reply: oneshot::Sender<Result<(), SecretStoreError>>,
    },
}

/// Sets `abandoned = true` if the Rust `SecretFuture` is dropped while queued or in flight.
struct AbandonOnDrop(Arc<AtomicBool>);

impl Drop for AbandonOnDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

/// SDK `SecretStore` adapter running synchronous foreign callbacks on a dedicated OS thread.
pub(crate) struct FfiSecretBridge {
    tx: mpsc::Sender<SecretJob>,
}

impl FfiSecretBridge {
    /// Spawns a dedicated `"papo-ffi-secret"` OS thread wrapping `store`.
    pub(crate) fn start(
        store: Arc<dyn FfiSecretStore>,
    ) -> Result<Arc<Self>, crate::errors::FfiClientError> {
        let (tx, rx) = mpsc::channel::<SecretJob>();
        let spawn_res = thread::Builder::new()
            .name("papo-ffi-secret".into())
            .spawn(move || worker_loop(store.as_ref(), rx));
        if spawn_res.is_err() {
            return Err(crate::errors::FfiClientError::Worker);
        }
        Ok(Arc::new(Self { tx }))
    }
}

/// Executes queued credential jobs serially off Tokio threads, skipping pre-start cancellations.
fn worker_loop(store: &dyn FfiSecretStore, rx: mpsc::Receiver<SecretJob>) {
    while let Ok(job) = rx.recv() {
        match job {
            SecretJob::Read {
                key,
                abandoned,
                reply,
            } => {
                if abandoned.load(Ordering::Acquire) {
                    continue;
                }
                let result = match catch_unwind(AssertUnwindSafe(|| store.read(key.into()))) {
                    Ok(Ok(read)) => {
                        let value = read.value.map(SecretValue::new);
                        Ok(if read.settled {
                            SecretRead::settled(value)
                        } else {
                            SecretRead::unfenced(value)
                        })
                    }
                    Ok(Err(err)) => Err(err.into()),
                    Err(_) => Err(SecretStoreError::Failed),
                };
                let _ = reply.send(result);
            }
            SecretJob::Write {
                key,
                value,
                abandoned,
                reply,
            } => {
                if abandoned.load(Ordering::Acquire) {
                    continue;
                }
                let bytes = value.expose().to_vec();
                let result = match catch_unwind(AssertUnwindSafe(|| store.write(key.into(), bytes)))
                {
                    Ok(res) => res.map_err(SecretStoreError::from),
                    Err(_) => Err(SecretStoreError::Failed),
                };
                let _ = reply.send(result);
            }
            SecretJob::Delete {
                key,
                abandoned,
                reply,
            } => {
                if abandoned.load(Ordering::Acquire) {
                    continue;
                }
                let result = match catch_unwind(AssertUnwindSafe(|| store.delete(key.into()))) {
                    Ok(res) => res.map_err(SecretStoreError::from),
                    Err(_) => Err(SecretStoreError::Failed),
                };
                let _ = reply.send(result);
            }
        }
    }
}

impl SecretStore for FfiSecretBridge {
    fn read(&self, key: SecretKey) -> SecretFuture<'_, SecretRead> {
        let tx = self.tx.clone();
        Box::pin(async move {
            let abandoned = Arc::new(AtomicBool::new(false));
            let _abandon = AbandonOnDrop(Arc::clone(&abandoned));
            let (reply, rx) = oneshot::channel();
            tx.send(SecretJob::Read {
                key,
                abandoned,
                reply,
            })
            .map_err(|_| SecretStoreError::Unavailable)?;
            rx.await.map_err(|_| SecretStoreError::Unavailable)?
        })
    }

    fn write(&self, key: SecretKey, value: SecretValue) -> SecretFuture<'_, ()> {
        let tx = self.tx.clone();
        Box::pin(async move {
            let abandoned = Arc::new(AtomicBool::new(false));
            let _abandon = AbandonOnDrop(Arc::clone(&abandoned));
            let (reply, rx) = oneshot::channel();
            tx.send(SecretJob::Write {
                key,
                value,
                abandoned,
                reply,
            })
            .map_err(|_| SecretStoreError::Unavailable)?;
            rx.await.map_err(|_| SecretStoreError::Unavailable)?
        })
    }

    fn delete(&self, key: SecretKey) -> SecretFuture<'_, ()> {
        let tx = self.tx.clone();
        Box::pin(async move {
            let abandoned = Arc::new(AtomicBool::new(false));
            let _abandon = AbandonOnDrop(Arc::clone(&abandoned));
            let (reply, rx) = oneshot::channel();
            tx.send(SecretJob::Delete {
                key,
                abandoned,
                reply,
            })
            .map_err(|_| SecretStoreError::Unavailable)?;
            rx.await.map_err(|_| SecretStoreError::Unavailable)?
        })
    }
}
