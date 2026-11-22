//! Server configuration, metadata refresh, and batch receipt FFI records.
//! Channel permission records live in `operations_channel_records.rs`; execution in `operations_admin.rs`.

use papo_sdk::{
    GenericResponse, Server,
    operations::{MetadataScope, OperationError, OperationReceipt, RefreshStatus},
};

use crate::{
    errors::FfiClientError,
    ids::{FfiAccountId, parse_uuid},
    operation_errors::{FfiOperationError, FfiRefreshStatus},
};

/// Bounded metadata scope eligible for independent post-mutation refresh recovery.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiMetadataScope {
    /// Server metadata excluding icon bytes.
    Server,
    /// Backend channel list and role overrides.
    Channels,
    /// Role definitions and member role labels.
    Roles,
    /// DM visibility and outbound blocks.
    Social,
    /// Account backend notification policy.
    AccountSettings,
    /// Single user profile and lightweight member summary.
    Profile {
        /// Target user UUID string.
        user_id: String,
    },
    /// Lightweight member summary without profile fetch.
    Member {
        /// Target user UUID string.
        user_id: String,
    },
    /// Canonical pinned message list for one channel.
    Pins {
        /// Channel UUID string.
        channel_id: String,
    },
}

impl FfiMetadataScope {
    /// Converts FFI metadata scope into validated SDK `MetadataScope`.
    ///
    /// # Errors
    /// Returns [`FfiOperationError::InvalidInput`] on malformed UUIDs.
    pub(crate) fn into_sdk(self) -> Result<MetadataScope, FfiOperationError> {
        let map_id = |raw: &str| parse_uuid(raw).map_err(|_| FfiOperationError::InvalidInput);
        Ok(match self {
            Self::Server => MetadataScope::Server,
            Self::Channels => MetadataScope::Channels,
            Self::Roles => MetadataScope::Roles,
            Self::Social => MetadataScope::Social,
            Self::AccountSettings => MetadataScope::AccountSettings,
            Self::Profile { user_id } => MetadataScope::Profile(map_id(&user_id)?),
            Self::Member { user_id } => MetadataScope::Member(map_id(&user_id)?),
            Self::Pins { channel_id } => MetadataScope::Pins(map_id(&channel_id)?),
        })
    }
}

/// Generation-tagged refresh status outcome returned by `refresh_metadata`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRefreshOutcome {
    /// Connection epoch executing the refresh.
    pub generation: u64,
    /// Resulting refresh status or warning.
    pub status: FfiRefreshStatus,
}

/// Receipt acknowledging a server mutation returning unit `()`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiUnitReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Connection generation dispatching the mutation.
    pub generation: u64,
    /// Independent post-ack local refresh status.
    pub refresh: FfiRefreshStatus,
}

impl FfiUnitReceipt {
    /// Maps an `OperationReceipt<()>` across FFI.
    pub(crate) fn from_sdk(receipt: OperationReceipt<()>) -> Self {
        Self {
            account: FfiAccountId::from_sdk(receipt.account),
            generation: receipt.generation.get(),
            refresh: receipt.refresh.into(),
        }
    }
}

/// Receipt acknowledging a server mutation returning `GenericResponse`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiGenericReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Connection generation dispatching the mutation.
    pub generation: u64,
    /// Optional textual response message.
    pub response: Option<String>,
    /// Independent post-ack local refresh status.
    pub refresh: FfiRefreshStatus,
}

impl From<OperationReceipt<GenericResponse>> for FfiGenericReceipt {
    fn from(receipt: OperationReceipt<GenericResponse>) -> Self {
        Self {
            account: FfiAccountId::from_sdk(receipt.account),
            generation: receipt.generation.get(),
            response: receipt.value.response,
            refresh: receipt.refresh.into(),
        }
    }
}

/// Per-item outcome inside `delete_*_batch`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiBatchUnitOutcome {
    /// Item deletion succeeded.
    Ok {
        /// Acknowledgment receipt for this item.
        receipt: FfiUnitReceipt,
    },
    /// Item deletion failed or was ambiguous without blocking sibling items.
    Err {
        /// Typed operation error.
        error: FfiOperationError,
    },
}

impl FfiBatchUnitOutcome {
    /// Converts one SDK batch element result.
    pub(crate) fn from_sdk(res: Result<OperationReceipt<()>, OperationError>) -> Self {
        match res {
            Ok(receipt) => Self::Ok {
                receipt: FfiUnitReceipt::from_sdk(receipt),
            },
            Err(err) => Self::Err { error: err.into() },
        }
    }
}

/// Per-item outcome inside `set_banned_batch`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiBatchGenericOutcome {
    /// Item succeeded.
    Ok {
        /// Acknowledgment receipt.
        receipt: FfiGenericReceipt,
    },
    /// Item failed or outcome is unknown.
    Err {
        /// Typed error.
        error: FfiOperationError,
    },
}

impl FfiBatchGenericOutcome {
    /// Converts one SDK generic batch result.
    pub(crate) fn from_sdk(res: Result<OperationReceipt<GenericResponse>, OperationError>) -> Self {
        match res {
            Ok(receipt) => Self::Ok {
                receipt: receipt.into(),
            },
            Err(err) => Self::Err { error: err.into() },
        }
    }
}

/// Partial server settings patch request.
#[derive(Clone, Debug, Default, PartialEq, Eq, uniffi::Record)]
pub struct FfiPatchServerRequest {
    /// Optional updated server display name.
    pub name: Option<String>,
    /// Optional base64-encoded icon payload.
    pub icon_blob: Option<String>,
    /// Optional icon format string.
    pub icon_format: Option<String>,
    /// Optional replacement server password.
    pub password: Option<String>,
    /// Optional public visibility flag.
    pub public_server: Option<bool>,
}

/// Full server settings replacement request.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiReplaceServerRequest {
    /// Server display name.
    pub name: String,
    /// Encoded icon payload (empty clears).
    pub icon_blob: String,
    /// Icon format (empty clears).
    pub icon_format: String,
    /// Server password (empty clears).
    pub password: String,
    /// Public visibility flag.
    pub public_server: bool,
}

/// Authoritative server record returned by `patch_server` and `replace_server`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiServerWire {
    /// Server UUID string.
    pub id: Option<String>,
    /// Display name.
    pub name: Option<String>,
    /// Icon media format.
    pub icon_format: Option<String>,
    /// Owner user UUID string.
    pub owner_id: Option<String>,
    /// Owner username.
    pub owner_username: Option<String>,
    /// Public vs password-protected flag.
    pub public_server: Option<bool>,
    /// Exact backend creation timestamp.
    pub created_at: Option<String>,
    /// Reported channel count.
    pub channel_count: Option<i64>,
    /// Reported member count.
    pub member_count: Option<i64>,
    /// Reported role count.
    pub role_count: Option<i64>,
}

impl From<&Server> for FfiServerWire {
    fn from(s: &Server) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            icon_format: s.icon_format.clone(),
            owner_id: s.owner_id.clone(),
            owner_username: s.owner_username.clone(),
            public_server: s.public,
            created_at: s.created_at.clone(),
            channel_count: s.channel_count,
            member_count: s.member_count,
            role_count: s.role_count,
        }
    }
}

/// Receipt carrying updated server wire metadata.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiServerReceipt {
    /// Source account.
    pub account: FfiAccountId,
    /// Dispatching connection generation.
    pub generation: u64,
    /// Returned server record.
    pub server: FfiServerWire,
    /// Post-mutation refresh status.
    pub refresh: FfiRefreshStatus,
}

/// Parses an account session and wraps any missing/revoked session error into `FfiOperationError`.
pub(crate) fn operation_session(
    client: &crate::client::FfiClient,
    account: &FfiAccountId,
) -> Result<papo_sdk::Session, FfiOperationError> {
    client
        .session_by_id(account)
        .map_err(|error| FfiOperationError::Session { error })
}

/// Closed client fallback for `run_scoped` inside operations modules.
pub(crate) fn closed_operation_error() -> FfiOperationError {
    FfiOperationError::Session {
        error: FfiClientError::Closed,
    }
}

/// Receipt helper for generic refresh status formatting.
pub(crate) fn refresh_outcome(generation: u64, status: RefreshStatus) -> FfiRefreshOutcome {
    FfiRefreshOutcome {
        generation,
        status: status.into(),
    }
}
