//! Process-owned SDK `Client` wrapper, runtime bridge, and write-once native media engine factory slot.
//!
//! Session 16 links `papo-ffi` (`rlib`) + `papo-call` + narrow JNI `ANativeWindow` surface code
//! into a single Android `.so`, avoiding cross-`cdylib` Rust static duplication. `papo-ffi`'s
//! own `cdylib` remains media-free for SDK CI, `:sdk-proof`, and JVM host unit tests.

use std::{
    future::Future,
    path::PathBuf,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use papo_sdk::{
    CallConfig, CallHandle, Client, ClientConfig, ClientHandle, DataRoot, DiskBudget, GiphyConfig,
    MediaEngineFactory, Session,
};
use tokio::sync::oneshot;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    errors::{FfiClientError, MediaFactoryRegistrationError},
    ids::FfiAccountId,
    secret::{FfiSecretBridge, FfiSecretStore},
    shutdown_report::FfiShutdownReport,
    subscription::CallbackTracker,
};

/// Process-global write-once slot for pre-open media engine factory injection.
static MEDIA_FACTORY_SLOT: OnceLock<MediaFactorySlot> = OnceLock::new();

/// State recorded in the write-once media factory slot.
enum MediaFactorySlot {
    /// A factory was registered before any `FfiClient::open`.
    Registered(Arc<dyn MediaEngineFactory>),
    /// `FfiClient::open` ran without prior registration and sealed the slot.
    SealedEmpty,
}

/// Registers the single native `MediaEngineFactory` before opening `FfiClient` in Session 16's
/// combined native image (`papo-ffi` `rlib` + `papo-call` + JNI bridge).
///
/// Must never be called across separately loaded `cdylib`s; each `cdylib` has distinct statics.
/// Fails with [`MediaFactoryRegistrationError::AlreadyRegistered`] on a second registration, or
/// [`MediaFactoryRegistrationError::ClientAlreadyOpened`] once [`FfiClient::open`] has run.
///
/// # Errors
/// Returns [`MediaFactoryRegistrationError`] if a factory was already set or sealed by `open`.
pub fn register_media_engine_factory_once(
    factory: Arc<dyn MediaEngineFactory>,
) -> Result<(), MediaFactoryRegistrationError> {
    if MEDIA_FACTORY_SLOT
        .set(MediaFactorySlot::Registered(factory))
        .is_err()
    {
        return match MEDIA_FACTORY_SLOT.get() {
            Some(MediaFactorySlot::Registered(_)) => {
                Err(MediaFactoryRegistrationError::AlreadyRegistered)
            }
            Some(MediaFactorySlot::SealedEmpty) | None => {
                Err(MediaFactoryRegistrationError::ClientAlreadyOpened)
            }
        };
    }
    Ok(())
}

/// Seals the global media factory slot on first `FfiClient::open` and returns any registered factory.
fn seal_and_get_media_factory() -> Option<Arc<dyn MediaEngineFactory>> {
    match MEDIA_FACTORY_SLOT.get_or_init(|| MediaFactorySlot::SealedEmpty) {
        MediaFactorySlot::Registered(factory) => Some(Arc::clone(factory)),
        MediaFactorySlot::SealedEmpty => None,
    }
}

/// Configuration crossing FFI to open the single process-owned `Client`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiClientConfig {
    /// Private profile directory (`filesDir/papo` on Android) owned exclusively by one `Client`.
    pub data_root_path: String,
    /// Shared Tokio worker count (minimum 1, default 2).
    pub worker_threads: u32,
    /// Per-keystore credential deadline in milliseconds (positive).
    pub credential_timeout_ms: u64,
    /// Per-REST/gateway request deadline in milliseconds (positive).
    pub request_timeout_ms: u64,
    /// Profile-wide allocated disk ceiling in bytes.
    pub disk_budget_bytes: u64,
    /// Call join timeout in milliseconds.
    pub call_join_timeout_ms: u64,
    /// Call media drain deadline on leave/switch/shutdown in milliseconds.
    pub call_drain_timeout_ms: u64,
    /// Optional external GIPHY API key supplied by the host build.
    pub giphy_api_key: Option<String>,
    /// Permits plain HTTP over literal loopback addresses (`127.0.0.1`/`::1`) for proofs only.
    pub insecure_loopback: bool,
}

/// Cancels child async operation when caller drops the FFI `Future` (e.g., Kotlin coroutine cancel).
struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Process-owned SDK client root bridging Android Kotlin callers to `papo-sdk` and `papo-sdk-ui`.
#[derive(uniffi::Object)]
pub struct FfiClient {
    /// Owned client stored only for clean shutdown and drop; runtime handle lives outside lock.
    client: Mutex<Option<Client>>,
    /// Cloneable SDK service handle.
    pub(crate) handle: ClientHandle,
    /// Handle to the single Client Tokio runtime captured at `open`.
    pub(crate) runtime: tokio::runtime::Handle,
    /// Shutdown cancellation signal shared across FFI tasks and subscription forwarders.
    pub(crate) shutdown: CancellationToken,
    /// Tracked FFI Tokio tasks (excluding dedicated OS callback threads so stalled callbacks never hang runtime stop).
    pub(crate) tasks: TaskTracker,
    /// Counts active foreign callbacks across all subscriptions for bounded shutdown drain.
    pub(crate) callbacks: CallbackTracker,
}

impl FfiClient {
    /// Opens `FfiClient` with an explicit optional `MediaEngineFactory` (Session 16 single-image
    /// bootstrap and isolated Rust tests). Does not read or seal `MEDIA_FACTORY_SLOT`.
    ///
    /// Must be called outside any Tokio runtime.
    ///
    /// # Errors
    /// Returns validation, configuration, profile-lock, or worker failures.
    pub fn open_with_media_engine(
        config: FfiClientConfig,
        secret_store: Arc<dyn FfiSecretStore>,
        media_factory: Option<Arc<dyn MediaEngineFactory>>,
    ) -> Result<Arc<Self>, FfiClientError> {
        let sdk_config = build_sdk_config(&config, media_factory)?;
        let root = DataRoot::new(PathBuf::from(&config.data_root_path));
        let secrets = FfiSecretBridge::start(secret_store)?;
        let client = Client::open(root, secrets, sdk_config)?;
        let handle = client.handle();
        let runtime = client.block_on(async { tokio::runtime::Handle::current() })?;
        Ok(Arc::new(Self {
            client: Mutex::new(Some(client)),
            handle,
            runtime,
            shutdown: CancellationToken::new(),
            tasks: TaskTracker::new(),
            callbacks: CallbackTracker::default(),
        }))
    }

    /// Returns the exact SDK `CallHandle` for `(account, attempt)` so Session 16's native
    /// `ANativeWindow` surface bridge binds to the same active call instance.
    #[must_use]
    pub fn call_handle_for(&self, account: FfiAccountId, attempt: u64) -> Option<CallHandle> {
        if self.is_closed() {
            return None;
        }
        let account = account.into_sdk().ok()?;
        let session = self.handle.session(account).ok()?;
        let active = session.calls().active()?;
        (active.attempt().raw() == attempt).then_some(active)
    }

    /// Resolves an authenticated account `Session` from explicit `FfiAccountId` without global state.
    ///
    /// # Errors
    /// Returns [`FfiClientError::InvalidArgument`] or `Client::session` revocation errors.
    pub(crate) fn session_by_id(&self, account: &FfiAccountId) -> Result<Session, FfiClientError> {
        if self.is_closed() {
            return Err(FfiClientError::Closed);
        }
        let parsed = account.clone().into_sdk()?;
        self.handle.session(parsed).map_err(FfiClientError::from)
    }

    /// Spawns `work` onto `Client`'s single Tokio runtime, propagating cancellation if caller
    /// drops the async future or if client shutdown starts.
    pub(crate) fn run_scoped<T, E, F>(
        &self,
        closed_err: impl Fn() -> E + Send + Sync + 'static,
        work: F,
    ) -> impl Future<Output = Result<T, E>> + Send + 'static
    where
        T: Send + 'static,
        E: Send + 'static,
        F: Future<Output = Result<T, E>> + Send + 'static,
    {
        let shutdown = self.shutdown.clone();
        let runtime = self.runtime.clone();
        let tasks = self.tasks.clone();
        let closed_err = Arc::new(closed_err);
        async move {
            // Acquire task token BEFORE checking shutdown/tasks.is_closed() so no task can
            // slip onto the runtime after `shutdown()` invokes `tasks.close()`.
            let token = tasks.token();
            if shutdown.is_cancelled() || tasks.is_closed() {
                drop(token);
                return Err(closed_err());
            }
            let cancel = shutdown.child_token();
            let _drop_guard = CancelOnDrop(cancel.clone());
            let (tx, rx) = oneshot::channel();
            let task_cancel = cancel.clone();
            let spawn_closed = Arc::clone(&closed_err);
            runtime.spawn(async move {
                let _token = token;
                let outcome = tokio::select! {
                    biased;
                    () = task_cancel.cancelled() => Err(spawn_closed()),
                    result = work => result,
                };
                let _ = tx.send(outcome);
            });
            rx.await.unwrap_or_else(|_| Err(closed_err()))
        }
    }
}

#[uniffi::export]
impl FfiClient {
    /// Opens the single process-owned `Client`, sealing global media-factory registration first.
    ///
    /// Call off any Tokio runtime (e.g., from an Android background executor / `Dispatchers.IO`).
    ///
    /// # Errors
    /// Returns profile lock, schema, configuration, or worker startup failures.
    #[uniffi::constructor]
    pub fn open(
        config: FfiClientConfig,
        secret_store: Arc<dyn FfiSecretStore>,
    ) -> Result<Arc<Self>, FfiClientError> {
        let media_factory = seal_and_get_media_factory();
        Self::open_with_media_engine(config, secret_store, media_factory)
    }

    /// True once client shutdown has started or underlying runtime admission ended.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.shutdown.is_cancelled() || self.handle.is_closed()
    }

    /// Cancels subscriptions and async FFI work, shuts down SDK storage cleanly, waits up to
    /// `callback_drain_timeout_ms` for foreign subscription callbacks, and reports honest status.
    ///
    /// Call off any Tokio runtime thread.
    ///
    /// # Errors
    /// Returns [`FfiClientError::RuntimeContext`] if called inside Tokio, [`FfiClientError::Closed`]
    /// if already shut down, or storage shutdown failures.
    pub fn shutdown(
        &self,
        callback_drain_timeout_ms: u64,
    ) -> Result<FfiShutdownReport, FfiClientError> {
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(FfiClientError::RuntimeContext);
        }
        let mut client = self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .ok_or(FfiClientError::Closed)?;
        self.shutdown.cancel();
        self.tasks.close();
        self.runtime.block_on(self.tasks.wait());
        let callback_drain = self
            .callbacks
            .drain(Duration::from_millis(callback_drain_timeout_ms));
        let storage = client.shutdown()?;
        Ok(FfiShutdownReport {
            reads_drained: storage.reads_drained,
            state: (&storage.state).into(),
            shared: (&storage.shared).into(),
            callback_drain,
        })
    }
}

impl Drop for FfiClient {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.tasks.close();
        let _ = self
            .client
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
    }
}

/// Validates FFI configuration inputs and maps them to `papo_sdk::ClientConfig`.
fn build_sdk_config(
    config: &FfiClientConfig,
    media_factory: Option<Arc<dyn MediaEngineFactory>>,
) -> Result<ClientConfig, FfiClientError> {
    if config.data_root_path.trim().is_empty()
        || config.worker_threads == 0
        || config.credential_timeout_ms == 0
        || config.request_timeout_ms == 0
        || config.call_join_timeout_ms == 0
        || config.call_drain_timeout_ms == 0
    {
        return Err(FfiClientError::Configuration);
    }
    let giphy = config
        .giphy_api_key
        .clone()
        .map(|key| GiphyConfig::new(key).map_err(|_| FfiClientError::Configuration))
        .transpose()?;
    let mut sdk = ClientConfig::default();
    sdk.worker_threads =
        usize::try_from(config.worker_threads).map_err(|_| FfiClientError::Configuration)?;
    sdk.credential_timeout = Duration::from_millis(config.credential_timeout_ms);
    sdk.request_timeout = Duration::from_millis(config.request_timeout_ms);
    sdk.disk = DiskBudget {
        bytes: config.disk_budget_bytes,
        maintenance_interval: sdk.disk.maintenance_interval,
    };
    sdk.call = CallConfig {
        join_timeout: Duration::from_millis(config.call_join_timeout_ms),
        drain_timeout: Duration::from_millis(config.call_drain_timeout_ms),
        command_queue_capacity: sdk.call.command_queue_capacity,
    };
    sdk.giphy = giphy;
    sdk.media_engine = media_factory;
    sdk.insecure_loopback = config.insecure_loopback;
    Ok(sdk)
}
