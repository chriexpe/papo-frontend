//! Stable typed identifiers, timestamp conversions, and validated boundary parsers.
//! Zero secret or server content is formatted in diagnostic displays or parser errors.

use std::str::FromStr;

use papo_sdk::{AccountId, ServerId, Uuid};

/// Server-scoped account identity crossing FFI without hidden global state.
#[derive(Clone, Debug, PartialEq, Eq, Hash, uniffi::Record)]
pub struct FfiAccountId {
    /// Canonical lowercase hex or parsed server identifier string.
    pub server_id: String,
    /// Backend user UUID string.
    pub user_id: String,
}

impl FfiAccountId {
    /// Converts a Rust `AccountId` into its explicit FFI value.
    #[must_use]
    pub fn from_sdk(value: AccountId) -> Self {
        Self {
            server_id: value.server().to_string(),
            user_id: value.user().to_string(),
        }
    }

    /// Validates and parses into `papo_sdk::AccountId`.
    ///
    /// # Errors
    /// Returns [`crate::errors::FfiClientError::InvalidArgument`] if either component is malformed.
    pub fn into_sdk(self) -> Result<AccountId, crate::errors::FfiClientError> {
        let server = parse_server_id(&self.server_id)?;
        let user = parse_uuid(&self.user_id)?;
        Ok(AccountId::new(server, user))
    }
}

/// Validates and parses a canonical UUID string.
///
/// # Errors
/// Returns [`crate::errors::FfiClientError::InvalidArgument`] on malformed input.
pub(crate) fn parse_uuid(raw: &str) -> Result<Uuid, crate::errors::FfiClientError> {
    Uuid::parse_str(raw).map_err(|_| crate::errors::FfiClientError::InvalidArgument)
}

/// Validates and parses a canonical `ServerId` string.
///
/// # Errors
/// Returns [`crate::errors::FfiClientError::InvalidArgument`] on malformed input.
pub(crate) fn parse_server_id(raw: &str) -> Result<ServerId, crate::errors::FfiClientError> {
    ServerId::from_str(raw).map_err(|_| crate::errors::FfiClientError::InvalidArgument)
}

/// Parses an optional slice of UUID strings without dropping invalid entries silently.
///
/// # Errors
/// Returns [`crate::errors::FfiClientError::InvalidArgument`] if any item is malformed.
pub(crate) fn parse_uuid_list(
    raw: Vec<String>,
) -> Result<Vec<Uuid>, crate::errors::FfiClientError> {
    raw.into_iter().map(|item| parse_uuid(&item)).collect()
}

/// Parses a slice of FFI account identifiers cleanly.
///
/// # Errors
/// Returns [`crate::errors::FfiClientError::InvalidArgument`] if any item is malformed.
pub(crate) fn parse_account_list(
    raw: Vec<FfiAccountId>,
) -> Result<Vec<AccountId>, crate::errors::FfiClientError> {
    raw.into_iter().map(FfiAccountId::into_sdk).collect()
}

/// Validates a non-negative or bounded Kotlin list index against `i32::MAX`.
///
/// # Errors
/// Returns [`crate::errors::FfiClientError::IndexOutOfRange`] if `index` exceeds Kotlin `Int`.
pub(crate) fn checked_list_index(index: usize) -> Result<u32, crate::errors::FfiClientError> {
    let as_u32 =
        u32::try_from(index).map_err(|_| crate::errors::FfiClientError::IndexOutOfRange)?;
    let _as_i32 =
        i32::try_from(as_u32).map_err(|_| crate::errors::FfiClientError::IndexOutOfRange)?;
    Ok(as_u32)
}

/// FFI representation of a nullable wire text field (absent vs explicit null vs string).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiNullableText {
    /// Field is explicitly cleared to JSON `null`.
    Clear,
    /// Field is replaced with the supplied string value.
    Value {
        /// Replacement text payload.
        text: String,
    },
}

impl From<FfiNullableText> for papo_sdk::NullableText {
    fn from(value: FfiNullableText) -> Self {
        match value {
            FfiNullableText::Clear => Self::Null,
            FfiNullableText::Value { text } => Self::Value(text),
        }
    }
}

/// Backend notification setting mode preserved across FFI without stringly comparisons.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiBackendMode {
    /// Deliver all messages.
    All,
    /// Deliver only mentions, replies, and DMs.
    OnlyMentions,
    /// Suppress backend notifications.
    None,
    /// Unrecognized backend mode string preserved verbatim.
    Unknown {
        /// Raw wire string reported by the backend.
        raw: String,
    },
}

impl From<&papo_sdk::BackendMode> for FfiBackendMode {
    fn from(value: &papo_sdk::BackendMode) -> Self {
        match value {
            papo_sdk::BackendMode::All => Self::All,
            papo_sdk::BackendMode::OnlyMentions => Self::OnlyMentions,
            papo_sdk::BackendMode::Off => Self::None,
            papo_sdk::BackendMode::Unknown(raw) => Self::Unknown { raw: raw.clone() },
        }
    }
}

impl From<FfiBackendMode> for papo_sdk::BackendMode {
    fn from(value: FfiBackendMode) -> Self {
        match value {
            FfiBackendMode::All => Self::All,
            FfiBackendMode::OnlyMentions => Self::OnlyMentions,
            FfiBackendMode::None => Self::Off,
            FfiBackendMode::Unknown { raw } => Self::Unknown(raw),
        }
    }
}

/// Exact backend history ordering direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiHistoryOrder {
    /// Ascending `(created_at, id)`.
    Asc,
    /// Descending `(created_at, id)`.
    Desc,
}

impl From<papo_sdk::HistoryOrder> for FfiHistoryOrder {
    fn from(value: papo_sdk::HistoryOrder) -> Self {
        match value {
            papo_sdk::HistoryOrder::Asc => Self::Asc,
            papo_sdk::HistoryOrder::Desc => Self::Desc,
        }
    }
}

impl From<FfiHistoryOrder> for papo_sdk::HistoryOrder {
    fn from(value: FfiHistoryOrder) -> Self {
        match value {
            FfiHistoryOrder::Asc => Self::Asc,
            FfiHistoryOrder::Desc => Self::Desc,
        }
    }
}

/// Exact opaque backend history cursor tuple.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiHistoryCursor {
    /// Exact backend RFC3339 timestamp spelling.
    pub created_at: String,
    /// Exact backend message UUID string.
    pub id: String,
}

impl From<&papo_sdk::HistoryCursor> for FfiHistoryCursor {
    fn from(value: &papo_sdk::HistoryCursor) -> Self {
        Self {
            created_at: value.created_at.clone(),
            id: value.id.clone(),
        }
    }
}

impl From<FfiHistoryCursor> for papo_sdk::HistoryCursor {
    fn from(value: FfiHistoryCursor) -> Self {
        Self {
            created_at: value.created_at,
            id: value.id,
        }
    }
}
