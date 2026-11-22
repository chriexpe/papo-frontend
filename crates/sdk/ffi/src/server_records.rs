//! Registry discovery metadata, saved servers/accounts, removals, and diffs across FFI.
//! Server rail badges live in `server_badges_records.rs`; execution lives in `servers.rs`.

use papo_sdk::{
    AddedServer, SavedAccount, SavedServer, ServerDiscovery, ServerIcon,
    removal::{RemovalFailureKind, RemovalJob, RemovalStep, RemovalTarget},
};
use papo_sdk_ui::ServerListSnapshot;

use crate::{define_ffi_vector_diff, ids::FfiAccountId};

/// Decoded server icon bytes from discovery (`max 256 KiB`), never from a disk path.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiServerIcon {
    /// Declared image format (e.g. `png`), trimmed when non-empty.
    pub format: Option<String>,
    /// Encoded image bytes.
    pub bytes: Vec<u8>,
}

impl From<ServerIcon> for FfiServerIcon {
    fn from(value: ServerIcon) -> Self {
        Self {
            format: value.format,
            bytes: value.bytes,
        }
    }
}

/// Public server self-description returned by `discover` and `add_server`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiServerDiscovery {
    /// Server display name if supplied.
    pub name: Option<String>,
    /// False if the server requires a server-wide password before login/registration.
    pub public_server: bool,
    /// Decoded optional icon if present and valid.
    pub icon: Option<FfiServerIcon>,
}

impl From<ServerDiscovery> for FfiServerDiscovery {
    fn from(value: ServerDiscovery) -> Self {
        Self {
            name: value.name,
            public_server: value.public,
            icon: value.icon.map(Into::into),
        }
    }
}

/// Result of saving and probing a new server.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiAddedServer {
    /// Assigned canonical server identifier.
    pub server_id: String,
    /// Public discovery metadata returned by `/server`.
    pub discovery: FfiServerDiscovery,
}

impl From<AddedServer> for FfiAddedServer {
    fn from(value: AddedServer) -> Self {
        Self {
            server_id: value.id.to_string(),
            discovery: value.discovery.into(),
        }
    }
}

/// Persisted non-secret server registry entry.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiSavedServer {
    /// Canonical server identifier string.
    pub id: String,
    /// Normalized server address string.
    pub address: String,
    /// Caller-supplied registration timestamp in Unix microseconds.
    pub added_at_us: i64,
}

impl From<SavedServer> for FfiSavedServer {
    fn from(value: SavedServer) -> Self {
        Self {
            id: value.address.id().to_string(),
            address: value.address.as_str().to_owned(),
            added_at_us: value.added_at.0,
        }
    }
}

/// Persisted non-secret account registry entry.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiSavedAccount {
    /// Account identity `(server_id, user_id)`.
    pub id: FfiAccountId,
    /// Username from last sign-in.
    pub username: String,
    /// Last known display name.
    pub display_name: Option<String>,
    /// Last known avatar reference token.
    pub avatar_ref: Option<String>,
    /// True if currently selected active account for its server.
    pub active: bool,
    /// Registration timestamp in Unix microseconds.
    pub created_at_us: i64,
    /// Never-reused monotonic OS credential epoch.
    pub credential_generation: u64,
}

impl From<SavedAccount> for FfiSavedAccount {
    fn from(value: SavedAccount) -> Self {
        Self {
            id: FfiAccountId::from_sdk(value.id),
            username: value.username,
            display_name: value.display_name,
            avatar_ref: value.avatar_ref,
            active: value.active,
            created_at_us: value.created_at.0,
            credential_generation: value.credential_generation.get(),
        }
    }
}

/// Target identity of a durable pending removal job.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiRemovalTarget {
    /// Account cleanup only.
    Account {
        /// Account being removed.
        account: FfiAccountId,
    },
    /// Server cleanup after its accounts have been removed.
    Server {
        /// Server being removed.
        server_id: String,
    },
}

/// Next idempotent step for a durable pending removal job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiRemovalStep {
    /// Revoking active session handles and cache connections.
    StopSession,
    /// Deleting OS keystore credentials across generations.
    Secrets,
    /// Removing cache SQLite and sidecars.
    Cache,
    /// Removing account media refs.
    MediaRefs,
    /// Deleting durable state rows.
    StateRows,
    /// Deleting final registry/job row.
    Finalize,
}

/// Typed failure kind recorded on a durable pending removal job.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiRemovalFailureKind {
    /// Stopping active session failed.
    SessionStop,
    /// Credential host read/delete timed out.
    SecretTimeout,
    /// Step canceled.
    Canceled,
    /// Credential host failed definitively.
    SecretStore,
    /// Credential store remained unfenced (`Uncertain`).
    SecretUncertain,
    /// Generation conflict in credential boundary.
    CredentialState,
    /// Account cache I/O failure.
    CacheIo,
    /// Media reference cleanup failed.
    MediaRefs,
    /// Blocking file task failed.
    FileTask,
}

/// Durable recoverable removal job status.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRemovalJob {
    /// Monotonic durable job identity.
    pub id: i64,
    /// Account or server target.
    pub target: FfiRemovalTarget,
    /// Next step pending execution.
    pub step: FfiRemovalStep,
    /// Last typed failure until retry succeeds.
    pub failure: Option<FfiRemovalFailureKind>,
}

impl From<RemovalJob> for FfiRemovalJob {
    fn from(value: RemovalJob) -> Self {
        let target = match value.target {
            RemovalTarget::Account(account) => FfiRemovalTarget::Account {
                account: FfiAccountId::from_sdk(account),
            },
            RemovalTarget::Server(server) => FfiRemovalTarget::Server {
                server_id: server.to_string(),
            },
            _ => FfiRemovalTarget::Server {
                server_id: String::new(),
            },
        };
        let step = match value.step {
            RemovalStep::Secrets => FfiRemovalStep::Secrets,
            RemovalStep::Cache => FfiRemovalStep::Cache,
            RemovalStep::MediaRefs => FfiRemovalStep::MediaRefs,
            RemovalStep::StateRows => FfiRemovalStep::StateRows,
            RemovalStep::Finalize => FfiRemovalStep::Finalize,
            // StopSession and future steps map to StopSession.
            _ => FfiRemovalStep::StopSession,
        };
        let failure = value.failure.map(|kind| match kind {
            RemovalFailureKind::SessionStop => FfiRemovalFailureKind::SessionStop,
            RemovalFailureKind::SecretTimeout => FfiRemovalFailureKind::SecretTimeout,
            RemovalFailureKind::Canceled => FfiRemovalFailureKind::Canceled,
            RemovalFailureKind::SecretStore => FfiRemovalFailureKind::SecretStore,
            RemovalFailureKind::SecretUncertain => FfiRemovalFailureKind::SecretUncertain,
            RemovalFailureKind::CredentialState => FfiRemovalFailureKind::CredentialState,
            RemovalFailureKind::CacheIo => FfiRemovalFailureKind::CacheIo,
            RemovalFailureKind::MediaRefs => FfiRemovalFailureKind::MediaRefs,
            // FileTask and future failure variants map to FileTask.
            _ => FfiRemovalFailureKind::FileTask,
        });
        Self {
            id: value.id,
            target,
            step,
            failure,
        }
    }
}

define_ffi_vector_diff!(
    "Ordered edit operation for saved servers.",
    FfiServerDiff,
    FfiSavedServer
);
define_ffi_vector_diff!(
    "Ordered edit operation for saved accounts.",
    FfiAccountDiff,
    FfiSavedAccount
);
define_ffi_vector_diff!(
    "Ordered edit operation for pending removal jobs.",
    FfiRemovalDiff,
    FfiRemovalJob
);

/// Atomic initial registry baseline.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiServerListSnapshot {
    /// SDK registry commit version.
    pub version: u64,
    /// Saved servers in durable order.
    pub servers: Vec<FfiSavedServer>,
    /// Saved accounts across all servers.
    pub accounts: Vec<FfiSavedAccount>,
    /// Pending recoverable removals.
    pub removals: Vec<FfiRemovalJob>,
}

/// Ordered registry delta batch preserving atomic commit coherence across all three lists.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiServerListUpdate {
    /// Registry commit version after applying all three diff lists.
    pub version: u64,
    /// Server list diffs.
    pub server_diffs: Vec<FfiServerDiff>,
    /// Account list diffs.
    pub account_diffs: Vec<FfiAccountDiff>,
    /// Removal list diffs.
    pub removal_diffs: Vec<FfiRemovalDiff>,
}

/// Initial baseline and ordered diff stream types for per-server account subscriptions.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiAccountListSnapshot {
    /// Registry commit version represented by `accounts`.
    pub version: u64,
    /// Accounts belonging to the subscribed server in registry order.
    pub accounts: Vec<FfiSavedAccount>,
}

/// Ordered diff batch for one server's visible accounts.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiAccountListUpdate {
    /// Registry commit version represented after applying `diffs`.
    pub version: u64,
    /// Ordered diffs; source version skips emit `Reset`.
    pub diffs: Vec<FfiAccountDiff>,
}

/// Converts UI `ServerListSnapshot` into FFI snapshot.
pub(crate) fn map_registry_snapshot(
    version: u64,
    snap: ServerListSnapshot,
) -> FfiServerListSnapshot {
    FfiServerListSnapshot {
        version,
        servers: snap.servers.into_iter().map(Into::into).collect(),
        accounts: snap.accounts.into_iter().map(Into::into).collect(),
        removals: snap.removals.into_iter().map(Into::into).collect(),
    }
}
