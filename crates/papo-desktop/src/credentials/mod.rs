//! OS credential storage for core workers, with an asynchronous GPUI boundary.
//!
//! No credential is written to a plaintext file. Native failures preserve this
//! run's in-memory state and reach a foreground callback, not a log-only notice.
//! Keep one service alive for the application, across account replacements.
//! The caller owns translated user feedback; this module owns no auth screen.

mod bridge;
mod store;
#[cfg(test)]
mod tests;

use bridge::{Bridge, Operation, Request};
use gpui::{App, AppContext as _, Global, Task};
use papo_core::storage::{Secret, SecretStore};
use std::sync::{Arc, Weak};
use store::OsSecretStore;

/// The attempted OS operation. It contains no credential or server address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CredentialOperation {
    /// Retrieve a previously persisted value.
    Read,
    /// Persist a new value, retaining it in memory on failure.
    Write,
    /// Remove a persisted value, hiding it in memory even on failure.
    Delete,
}

/// Sanitized failures; native error strings never cross this boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    /// The OS service failed, was missing, or access was denied.
    #[error("OS credential storage is unavailable")]
    Unavailable,
    /// An existing entry did not match this adapter's format.
    #[error("OS credential entry has an invalid format")]
    InvalidEntry,
    /// A synchronous call would block GPUI's foreground thread.
    #[error("synchronous credential access requires a worker thread")]
    ForegroundCall,
    /// The service is stopping or its foreground owner was dropped.
    #[error("credential service is stopped")]
    Stopped,
    /// An internal synchronization invariant failed.
    #[error("credential service synchronization failed")]
    Internal,
}

/// Foreground failure notification, scoped to the original storage request.
///
/// The server is core's stable key, not a display label or session incarnation.
/// Before connecting real notices, add an operation-generation guard: matching
/// only this key cannot reject a delayed failure from a replaced account.
/// This type intentionally has no Debug implementation: server keys can contain
/// a readable address fragment.
pub struct CredentialFailure {
    server: String,
    secret: Secret,
    operation: CredentialOperation,
    error: CredentialError,
}

impl CredentialFailure {
    /// The core server key captured by the operation.
    #[must_use]
    pub fn server(&self) -> &str {
        &self.server
    }
    /// The kind of credential, never its value.
    #[must_use]
    pub fn secret(&self) -> Secret {
        self.secret
    }
    /// The operation which failed.
    #[must_use]
    pub fn operation(&self) -> CredentialOperation {
        self.operation
    }
    /// The sanitized cause, safe to format without exposing backend details.
    #[must_use]
    pub fn error(&self) -> CredentialError {
        self.error
    }
}

/// Retains the foreground bridge while core workers use its synchronous store.
///
/// Dropping or stopping it releases pending callers without waiting for GPUI.
/// A canceled native write/delete may already have reached the OS; cancellation
/// is not a persistence acknowledgement. Keep this owner across session changes.
pub struct CredentialService {
    store: Arc<OsSecretStore>,
    _foreground: Task<()>,
}

#[derive(Default)]
struct Services(Vec<Weak<Bridge>>);
impl Global for Services {}

type NativeRequest = fn(&Request, &App) -> Task<Result<Option<String>, CredentialError>>;

impl CredentialService {
    /// Creates the application-owned adapter on GPUI's foreground thread.
    ///
    /// `on_failure` runs on the foreground after the core reply is completed.
    /// It must not block GPUI or synchronously wait for account or worker work.
    /// Before enabling real notices, add session-incarnation/generation validation
    /// and translated user feedback; the server key alone cannot reject stale
    /// account failures. Logging alone does not fulfill the credential policy
    /// (§4.5).
    pub fn new(
        cx: &mut App,
        on_failure: impl FnMut(CredentialFailure, &mut App) + 'static,
    ) -> Self {
        Self::with_native(cx, on_failure, native_request)
    }

    fn with_native(
        cx: &mut App,
        mut on_failure: impl FnMut(CredentialFailure, &mut App) + 'static,
        dispatch: NativeRequest,
    ) -> Self {
        let (bridge, requests) = Bridge::new();
        cx.default_global::<Services>()
            .0
            .retain(|bridge| bridge.strong_count() > 0);
        cx.default_global::<Services>()
            .0
            .push(Arc::downgrade(&bridge));
        let weak = Arc::downgrade(&bridge);
        cx.on_app_quit(move |_| {
            if let Some(bridge) = weak.upgrade() {
                bridge.stop();
            }
            async {}
        })
        .detach();
        let foreground = cx.spawn(async move |cx| {
            while let Ok(request) = requests.recv().await {
                if request.is_canceled() {
                    continue;
                }
                let result = cx.update(|cx| dispatch(&request, cx)).await;
                if request.is_canceled() {
                    continue;
                }
                let failure = result.as_ref().err().map(|error| CredentialFailure {
                    server: request.server.clone(),
                    secret: request.secret,
                    operation: request.operation.kind(),
                    error: *error,
                });
                // Release the worker reply before a callback can initiate account
                // work or shutdown. The worker can then release its operation lock.
                request.complete(result);
                if let Some(failure) = failure {
                    cx.update(|cx| on_failure(failure, cx));
                }
            }
        });
        Self {
            store: Arc::new(OsSecretStore::new(bridge)),
            _foreground: foreground,
        }
    }

    /// A shared core adapter. Call its synchronous methods only from workers.
    #[must_use]
    pub fn store(&self) -> Arc<dyn SecretStore> {
        self.store.clone()
    }

    /// Rejects new native requests and releases outstanding callers immediately.
    /// Idempotent; invoke before any blocking shutdown join.
    pub fn stop(&self) {
        self.store.bridge.stop();
    }
}

impl Drop for CredentialService {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Closes credential waits before the session's native shutdown fence joins Net.
pub(crate) fn stop_before_session_join(cx: &App) {
    if let Some(services) = cx.try_global::<Services>() {
        for bridge in services.0.iter().filter_map(Weak::upgrade) {
            bridge.stop();
        }
    }
}

/// A stable username prevents duplicate Secret Service items when accounts change.
const USERNAME: &str = "papo-desktop";

fn native_request(request: &Request, cx: &App) -> Task<Result<Option<String>, CredentialError>> {
    match &request.operation {
        Operation::Read => {
            let task = cx.read_credentials(&request.key);
            cx.background_spawn(async move {
                let value = task.await.map_err(|_| CredentialError::Unavailable)?;
                decode(value)
            })
        }
        Operation::Write(value) => {
            let task = cx.write_credentials(&request.key, USERNAME, value.as_bytes());
            cx.background_spawn(async move {
                task.await
                    .map(|()| None)
                    .map_err(|_| CredentialError::Unavailable)
            })
        }
        Operation::Delete => {
            let task = cx.delete_credentials(&request.key);
            cx.background_spawn(async move {
                task.await
                    .map(|()| None)
                    .map_err(|_| CredentialError::Unavailable)
            })
        }
    }
}

fn decode(value: Option<(String, Vec<u8>)>) -> Result<Option<String>, CredentialError> {
    value
        .map(|(username, bytes)| {
            if username != USERNAME {
                return Err(CredentialError::InvalidEntry);
            }
            String::from_utf8(bytes).map_err(|_| CredentialError::InvalidEntry)
        })
        .transpose()
}
