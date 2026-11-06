//! OS credential storage for core workers, with an asynchronous GPUI boundary.
//!
//! No credential is written to a plaintext file. Native failures preserve this
//! run's in-memory state and reach a foreground callback, not a log-only notice.
//! Keep one service alive for the application, across account replacements.
//! The caller owns translated user feedback; this module owns no auth screen.

mod bridge;
mod generation;
#[cfg(test)]
mod generation_tests;
mod store;
#[cfg(test)]
mod tests;

use bridge::{Bridge, Operation, Request};
use generation::Generation;
use gpui::{App, AppContext as _, Global, Task};
use papo_core::storage::{Secret, SecretStore};
use std::{
    collections::HashMap,
    marker::PhantomData,
    rc::Rc,
    sync::{Arc, Weak},
};
use store::{OsSecretStore, Overlay};

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
    /// The handle belongs to a replaced or invalidated server incarnation.
    #[error("credential session is stale")]
    Stale,
    /// A scoped handle was used for a different server.
    #[error("credential handle belongs to another server")]
    WrongServer,
    /// An internal synchronization invariant failed.
    #[error("credential service synchronization failed")]
    Internal,
}

/// Foreground failure notification, scoped to the original storage request.
///
/// The original handle's generation is retained, even if presentation is deferred.
/// Call `is_current` on the foreground immediately before showing a notice; a
/// failure delivered earlier can become stale after replacement or logout.
/// This type intentionally has no Debug implementation: server keys can contain
/// a readable address fragment.
pub struct CredentialFailure {
    server: String,
    secret: Secret,
    operation: CredentialOperation,
    error: CredentialError,
    generation: Arc<Generation>,
}

impl CredentialFailure {
    /// Whether this request still belongs to the server's current incarnation.
    ///
    /// This is a snapshot, not a lease. Recheck on the foreground immediately
    /// before presentation, after any await or queue boundary. Replacement,
    /// invalidation, service drop and shutdown make it false permanently.
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.generation.is_current()
    }

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
/// The service is foreground-only; only its stores and failures cross threads.
///
/// ```compile_fail
/// use papo_desktop::credentials::CredentialService;
/// fn move_to_worker(service: CredentialService) {
///     std::thread::spawn(move || drop(service));
/// }
/// ```
pub struct CredentialService {
    bridge: Arc<Bridge>,
    overlay: Arc<Overlay>,
    generations: HashMap<String, Arc<Generation>>,
    _foreground: Task<()>,
    // Invalidation and dispatch/callback checks must share the foreground thread.
    _foreground_only: PhantomData<Rc<()>>,
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
    /// Only current generations reach the callback. Deferred presentation must
    /// recheck `CredentialFailure::is_current` immediately before showing it.
    /// The caller still owns translated user feedback; logging alone does not
    /// fulfill the credential policy (§4.5).
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
                let result = cx
                    .update(|cx| dispatch_current(&request, cx, dispatch))
                    .await;
                if request.is_canceled() {
                    continue;
                }
                let failure = result.as_ref().err().map(|error| CredentialFailure {
                    server: request.server.clone(),
                    secret: request.secret,
                    operation: request.operation.kind(),
                    error: *error,
                    generation: Arc::clone(&request.generation),
                });
                // Release the worker reply before a callback can initiate account
                // work or shutdown. The worker can then release its operation lock.
                request.complete(result);
                if let Some(failure) = failure {
                    cx.update(|cx| {
                        if failure.is_current() {
                            on_failure(failure, cx);
                        }
                    });
                }
            }
        });
        Self {
            bridge,
            overlay: Arc::default(),
            generations: HashMap::new(),
            _foreground: foreground,
            _foreground_only: PhantomData,
        }
    }

    /// Begins a new credential incarnation for one core server key.
    ///
    /// Call on the foreground at each session replacement, including a repeat login of
    /// the same account. This is not a getter: it revokes the previous handle
    /// for this server. Retain and clone the returned Arc within that session.
    /// Other servers and this run's memory overlay/tombstones are unchanged.
    /// Synchronous methods are worker-only and reject other server keys.
    #[must_use]
    pub fn replace_store(&mut self, server: &str) -> Arc<dyn SecretStore> {
        self.invalidate(server);
        let generation = Arc::new(Generation::new(server, &self.bridge));
        self.generations
            .insert(server.to_owned(), Arc::clone(&generation));
        Arc::new(OsSecretStore::new(
            Arc::clone(&self.bridge),
            generation,
            Arc::clone(&self.overlay),
        ))
    }

    /// Revokes a server's handle and notices, without waiting for native I/O.
    ///
    /// Invoke on the foreground at logout/removal, or use `replace_store` for
    /// replacement. This does not delete credentials or roll back OS mutations. Coordinate
    /// deletion before revocation or through a fresh cleanup incarnation.
    pub fn invalidate(&mut self, server: &str) {
        if let Some(generation) = self.generations.remove(server) {
            generation.invalidate();
        }
    }

    /// Rejects new native requests and releases outstanding callers immediately.
    /// Idempotent; invoke on the foreground before any blocking shutdown join.
    pub fn stop(&self) {
        self.bridge.stop();
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

/// Checks on the foreground without an await between validation and dispatch.
fn dispatch_current(
    request: &Request,
    cx: &App,
    dispatch: NativeRequest,
) -> Task<Result<Option<String>, CredentialError>> {
    if request.generation.is_current() {
        dispatch(request, cx)
    } else {
        Task::ready(Err(CredentialError::Stale))
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
