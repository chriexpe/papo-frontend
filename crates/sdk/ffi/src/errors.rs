//! Lossless core, credential, discovery, and authentication error enums across FFI.
//! Media and disk errors live in `media_disk_errors.rs`; service errors live in
//! `timeline_errors.rs` and `operation_errors.rs`.

use papo_sdk::{
    ApiError, AuthFailure, ClientError, ProblemKind, StorageError,
    secret::{SecretError, SecretFailure, SecretStoreError},
};

/// Failure returned by write-once native media engine factory registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum MediaFactoryRegistrationError {
    /// A media engine factory was already registered before this call.
    #[error("media engine factory already registered")]
    AlreadyRegistered,
    /// `FfiClient::open` already sealed process-global factory registration.
    #[error("client already opened before media engine registration")]
    ClientAlreadyOpened,
}

/// Typed credential-store failure or uncertainty reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiSecretFailure {
    /// Platform read or write timed out.
    Timeout,
    /// Caller canceled credential access.
    Canceled,
    /// Platform store is temporarily unavailable.
    Unavailable,
    /// Platform store denied access.
    AccessDenied,
    /// Platform operation failed; effects remain uncertain.
    Failed,
    /// Platform returned `settled = false`, proving no quiescence fence.
    Unfenced,
}

impl From<SecretFailure> for FfiSecretFailure {
    fn from(value: SecretFailure) -> Self {
        match value {
            SecretFailure::Timeout => Self::Timeout,
            SecretFailure::Canceled => Self::Canceled,
            SecretFailure::Store(SecretStoreError::Unavailable) => Self::Unavailable,
            SecretFailure::Store(SecretStoreError::AccessDenied) => Self::AccessDenied,
            SecretFailure::Unfenced => Self::Unfenced,
            SecretFailure::Store(_) | _ => Self::Failed,
        }
    }
}

/// Typed SDK credential access or generation error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiSecretError {
    /// Read failed cleanly without starting an unresolved mutation.
    #[error("credential store access failed: {reason:?}")]
    Failure {
        /// Categorized host failure reason.
        reason: FfiSecretFailure,
    },
    /// Mutation outcome cannot yet be proved settled.
    #[error("credential store outcome uncertain: {reason:?}")]
    Uncertain {
        /// Categorized host uncertainty reason.
        reason: FfiSecretFailure,
    },
    /// Credential generation was superseded by a newer epoch.
    #[error("credential generation superseded")]
    Superseded,
    /// Another credential access was already admitted for this account generation.
    #[error("credential generation busy")]
    Busy,
    /// Account has not been activated in durable storage.
    #[error("credential account not activated")]
    UnknownAccount,
    /// Internal synchronization poisoned before OS dispatch.
    #[error("credential state unavailable")]
    Poisoned,
}

impl From<SecretError> for FfiSecretError {
    fn from(value: SecretError) -> Self {
        match value {
            SecretError::Failure(reason) => Self::Failure {
                reason: reason.into(),
            },
            SecretError::Uncertain(reason) => Self::Uncertain {
                reason: reason.into(),
            },
            SecretError::Superseded => Self::Superseded,
            SecretError::Busy => Self::Busy,
            SecretError::UnknownAccount => Self::UnknownAccount,
            SecretError::Poisoned => Self::Poisoned,
            _ => Self::Failure {
                reason: FfiSecretFailure::Failed,
            },
        }
    }
}

/// Classification for `add_server` and `discover` failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiDiscoveryFailureKind {
    /// Server reported it has not completed setup.
    NotSetUp,
    /// Endpoint responded with non-Papo content.
    NotPapo,
    /// Transport/timeout/5xx/rate-limit failure.
    Unreachable,
}

/// Classification for explicit authentication operation failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiAuthFailureKind {
    /// Server password required or lapsed.
    ServerAccessRequired,
    /// Account password or server password rejected.
    InvalidCredentials,
    /// Account banned.
    Banned,
    /// Requested username already taken.
    UsernameTaken,
    /// Rate limited by server.
    RateLimited,
    /// Server failed or unreachable.
    NoResponse,
}

/// Machine-readable problem classification from `papo-api`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiProblemKind {
    /// Resource not found.
    NotFound,
    /// Authentication failed or missing.
    Unauthorized,
    /// Server-wide password required.
    ServerAccessRequired,
    /// Account/server credentials rejected.
    InvalidCredentials,
    /// Account banned.
    Banned,
    /// Username already taken during registration.
    UsernameTaken,
    /// Another defined or unrecognized problem kind.
    Other,
}

impl From<ProblemKind> for FfiProblemKind {
    fn from(value: ProblemKind) -> Self {
        match value {
            ProblemKind::NotFound => Self::NotFound,
            ProblemKind::Unauthorized => Self::Unauthorized,
            ProblemKind::ServerAccessRequired => Self::ServerAccessRequired,
            ProblemKind::InvalidCredentials => Self::InvalidCredentials,
            ProblemKind::Banned => Self::Banned,
            ProblemKind::UsernameTaken => Self::UsernameTaken,
            _ => Self::Other,
        }
    }
}

/// Sanitized client and transport lifecycle error.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiClientError {
    /// Caller supplied malformed UUID, ID, path, or boundary input.
    #[error("invalid argument")]
    InvalidArgument,
    /// Index exceeds Kotlin signed 32-bit list bounds.
    #[error("list index out of range")]
    IndexOutOfRange,
    /// Profile lock already held by another client instance.
    #[error("profile already in use")]
    ProfileInUse,
    /// Lifecycle method requiring non-Tokio thread was called inside Tokio.
    #[error("lifecycle call inside runtime context")]
    RuntimeContext,
    /// Client executor failed or stopped.
    #[error("client executor failed")]
    Executor,
    /// Client or session closed cleanly.
    #[error("client or session closed")]
    Closed,
    /// Operation superseded by newer generation.
    #[error("session generation superseded")]
    Superseded,
    /// Requested entity or account not found.
    #[error("not found")]
    NotFound,
    /// Account identity mismatch.
    #[error("account identity mismatch")]
    Identity,
    /// Invalid resource payload.
    #[error("invalid resource response")]
    ResourceData,
    /// Authoritative resource batch failed admission.
    #[error("resource admission rejected")]
    ResourceRejected,
    /// Profile request limit exceeded.
    #[error("profile request limit exceeded")]
    ProfileLimit,
    /// Event journal overflow required reload.
    #[error("reconcile overflow")]
    ReconcileOverflow,
    /// Resource reload required.
    #[error("reconcile required")]
    ReconcileRequired,
    /// History layout invariant violated cleanly.
    #[error("history layout error")]
    HistoryLayout,
    /// Rebuildable cache storage failed.
    #[error("account cache failed")]
    Cache,
    /// Invalid auth payload or stored credentials.
    #[error("invalid auth data")]
    InvalidAuth,
    /// Account signed out.
    #[error("signed out")]
    SignedOut,
    /// Network or storage deadline expired.
    #[error("timed out")]
    Timeout,
    /// Invalid client configuration.
    #[error("invalid configuration")]
    Configuration,
    /// Invalid server URL/address.
    #[error("invalid server address")]
    Address,
    /// Durable state storage failure.
    #[error("durable storage failure")]
    Storage,
    /// Credential host failure.
    #[error("credential failure")]
    Secret,
    /// Backend HTTP status failure.
    #[error("backend http status {status}")]
    ApiHttp {
        /// HTTP status code returned by backend.
        status: u16,
    },
    /// Backend RFC9457 problem response without leaking message bodies.
    #[error("backend problem {status}")]
    ApiProblem {
        /// HTTP status code.
        status: u16,
        /// Classified problem kind.
        kind: FfiProblemKind,
    },
    /// Network or decode failure in REST client.
    #[error("transport or decode failure")]
    ApiTransport,
    /// Single-socket gateway failure.
    #[error("gateway failure")]
    Gateway,
    /// Discovery failed with classified reason.
    #[error("discovery failed")]
    Discovery {
        /// Classified discovery failure category.
        reason: FfiDiscoveryFailureKind,
    },
    /// Registry entry already existed.
    #[error("registry entry already exists")]
    AlreadyExists,
    /// Explicit ordering list did not match visible registry entries.
    #[error("invalid registry order")]
    InvalidOrder,
    /// Host OS thread spawn failed or panicked.
    #[error("internal worker failure")]
    Worker,
}

impl From<&ClientError> for FfiClientError {
    fn from(value: &ClientError) -> Self {
        match value {
            ClientError::ProfileInUse => Self::ProfileInUse,
            ClientError::RuntimeContext => Self::RuntimeContext,
            ClientError::Executor => Self::Executor,
            ClientError::Closed => Self::Closed,
            ClientError::Superseded => Self::Superseded,
            ClientError::NotFound | ClientError::Storage(StorageError::NotFound) => Self::NotFound,
            ClientError::Identity => Self::Identity,
            ClientError::ResourceData => Self::ResourceData,
            ClientError::ResourceRejected(_) => Self::ResourceRejected,
            ClientError::ProfileLimit => Self::ProfileLimit,
            ClientError::ReconcileOverflow => Self::ReconcileOverflow,
            ClientError::ReconcileRequired => Self::ReconcileRequired,
            ClientError::HistoryLayout => Self::HistoryLayout,
            ClientError::Cache(_) => Self::Cache,
            ClientError::InvalidAuth => Self::InvalidAuth,
            ClientError::SignedOut => Self::SignedOut,
            ClientError::Timeout => Self::Timeout,
            ClientError::Configuration => Self::Configuration,
            ClientError::Address(_) => Self::Address,
            ClientError::Storage(StorageError::AlreadyExists) => Self::AlreadyExists,
            ClientError::Storage(StorageError::InvalidOrder) => Self::InvalidOrder,
            ClientError::Secret(_) => Self::Secret,
            ClientError::Api(ApiError::Http { status }) => Self::ApiHttp { status: *status },
            ClientError::Api(ApiError::Problem { status, problem }) => Self::ApiProblem {
                status: *status,
                kind: problem.kind().into(),
            },
            ClientError::Api(_) => Self::ApiTransport,
            ClientError::Gateway(_) => Self::Gateway,
            // Storage and unknown future ClientError variants surface as Storage.
            ClientError::Storage(_) | _ => Self::Storage,
        }
    }
}

impl From<ClientError> for FfiClientError {
    fn from(value: ClientError) -> Self {
        Self::from(&value)
    }
}

impl From<StorageError> for FfiClientError {
    fn from(value: StorageError) -> Self {
        Self::from(&ClientError::from(value))
    }
}

/// Converts a discovery error preserving typed `DiscoveryFailure` categories.
pub(crate) fn map_discovery_error(error: &ClientError) -> FfiClientError {
    if let Some(kind) = papo_sdk::DiscoveryFailure::classify(error) {
        let reason = match kind {
            papo_sdk::DiscoveryFailure::NotSetUp => FfiDiscoveryFailureKind::NotSetUp,
            papo_sdk::DiscoveryFailure::NotPapo => FfiDiscoveryFailureKind::NotPapo,
            // Unreachable and future discovery failures map to Unreachable.
            _ => FfiDiscoveryFailureKind::Unreachable,
        };
        return FfiClientError::Discovery { reason };
    }
    FfiClientError::from(error)
}

/// Typed explicit authentication failure for login, register, and server password.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error, uniffi::Error)]
pub enum FfiAuthError {
    /// Classified authentication failure reason.
    #[error("authentication failed")]
    Failure {
        /// Categorized user-actionable auth reason.
        reason: FfiAuthFailureKind,
    },
    /// Other underlying client/lifecycle failure.
    #[error("client auth lifecycle failure")]
    Client {
        /// Underlying typed client failure.
        error: FfiClientError,
    },
}

impl From<ClientError> for FfiAuthError {
    fn from(error: ClientError) -> Self {
        if let Some(kind) = AuthFailure::classify(&error) {
            let reason = match kind {
                AuthFailure::ServerAccessRequired => FfiAuthFailureKind::ServerAccessRequired,
                AuthFailure::InvalidCredentials => FfiAuthFailureKind::InvalidCredentials,
                AuthFailure::Banned => FfiAuthFailureKind::Banned,
                AuthFailure::UsernameTaken => FfiAuthFailureKind::UsernameTaken,
                AuthFailure::RateLimited => FfiAuthFailureKind::RateLimited,
                // NoResponse and future auth failure reasons map to NoResponse.
                _ => FfiAuthFailureKind::NoResponse,
            };
            return Self::Failure { reason };
        }
        Self::Client {
            error: FfiClientError::from(&error),
        }
    }
}
