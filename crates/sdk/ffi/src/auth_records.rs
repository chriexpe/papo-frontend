//! Authentication state enums, response DTOs, and foreign listener traits across FFI.
//! Stateful bootstrap and account handles live in `auth.rs`.

use papo_sdk::{AuthState, ConnectedDevicesResponse, DropConnectionResponse};

/// Account authentication status independent of transport connection state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiAuthState {
    /// Server-wide shared password is required first.
    NeedsServerPassword,
    /// Signed out locally.
    SignedOut,
    /// Login attempt is currently in progress.
    SigningIn,
    /// Saved credentials exist but server cannot yet verify identity.
    NeedsServer,
    /// Identity verified against server.
    Ready,
    /// Server rejected saved credentials; explicit sign-in required.
    Expired,
}

impl From<AuthState> for FfiAuthState {
    fn from(value: AuthState) -> Self {
        match value {
            AuthState::NeedsServerPassword => Self::NeedsServerPassword,
            AuthState::SigningIn => Self::SigningIn,
            AuthState::NeedsServer => Self::NeedsServer,
            AuthState::Ready => Self::Ready,
            AuthState::Expired => Self::Expired,
            // SignedOut and unknown future states map cleanly to SignedOut.
            _ => Self::SignedOut,
        }
    }
}

/// Registration response returned by `FfiAuthHandle::register`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiRegisterResponse {
    /// Assigned user UUID if returned by server.
    pub id: Option<String>,
    /// Registered username echoed by server.
    pub username: Option<String>,
    /// Creation timestamp string echoed by server.
    pub created_at: Option<String>,
}

/// Single active connection entry returned by `connected_devices`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiConnectedDevice {
    /// Connection UUID if returned.
    pub id: Option<String>,
    /// Reported creation timestamp string.
    pub created_at: Option<String>,
    /// Expiration timestamp string.
    pub expires_at: Option<String>,
}

/// Generation-verified list of active connected devices on an account.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiConnectedDevicesResult {
    /// Connection generation that executed the read.
    pub generation: u64,
    /// Active connections reported by backend.
    pub connections: Vec<FfiConnectedDevice>,
}

/// Result of dropping one remote connection.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiDropConnectionResult {
    /// Connection generation that executed the mutation.
    pub generation: u64,
    /// Count of dropped connections if returned.
    pub dropped_count: Option<i64>,
}

/// Listener for authentication state changes on one `FfiAuthHandle`.
#[uniffi::export(with_foreign)]
pub trait FfiAuthStateListener: Send + Sync {
    /// Initial atomic atomic auth state.
    fn on_snapshot(&self, state: FfiAuthState);
    /// Subsequent auth state changes until revocation/closure.
    fn on_update(&self, state: FfiAuthState);
    /// Terminal notification when auth handle closes.
    fn on_closed(&self);
}

/// Converts `papo_api::wire::ConnectedDevicesResponse` to FFI result.
pub(crate) fn map_connected_devices(
    generation: u64,
    response: ConnectedDevicesResponse,
) -> FfiConnectedDevicesResult {
    let connections = response
        .connections
        .unwrap_or_default()
        .into_iter()
        .map(|row| FfiConnectedDevice {
            id: row.id,
            created_at: row.created_at,
            expires_at: row.expires_at,
        })
        .collect();
    FfiConnectedDevicesResult {
        generation,
        connections,
    }
}

/// Converts `papo_api::wire::DropConnectionResponse` to FFI result.
pub(crate) fn map_drop_connection(
    generation: u64,
    response: DropConnectionResponse,
) -> FfiDropConnectionResult {
    FfiDropConnectionResult {
        generation,
        dropped_count: response.dropped,
    }
}
