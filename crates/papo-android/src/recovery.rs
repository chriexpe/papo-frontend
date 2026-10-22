use crate::bindings::{Backend, CoreError, NativeClient};
use papo_core::{api::client::{Api, ApiError, Session}, recovery};
use std::sync::Arc;

#[derive(Clone, uniffi::Record)]
pub struct RecoveryReceipt {
    pub url: String,
    pub expires_at: String,
}

/// Semantic failures cross UniFFI; presentation wording belongs to Android.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum RecoveryError {
    #[error("invalid_link")] InvalidLink,
    #[error("invalid_server")] InvalidServer,
    #[error("wrong_server")] WrongServer,
    #[error("empty_password")] EmptyPassword,
    #[error("not_server_owner")] NotOwner,
    #[error("invalid_member")] InvalidMember,
    #[error("account_changed")] AccountChanged,
    #[error("network")] Network,
    #[error("rate_limited")] RateLimited,
    #[error("unavailable")] Unavailable,
    #[error("{details}")] Server {details: String},
}
impl From<recovery::RecoveryLinkError> for RecoveryError {
    fn from(value: recovery::RecoveryLinkError) -> Self {
        match value {
            recovery::RecoveryLinkError::InvalidLink => Self::InvalidLink,
            recovery::RecoveryLinkError::InvalidServer => Self::InvalidServer,
            recovery::RecoveryLinkError::WrongServer => Self::WrongServer,
        }
    }
}
impl From<CoreError> for RecoveryError {
    fn from(_: CoreError) -> Self {Self::AccountChanged}
}
impl From<ApiError> for RecoveryError {
    fn from(error: ApiError) -> Self {
        match error {
            ApiError::Network(_) => Self::Network,
            ApiError::TooManyRequests(_) => Self::RateLimited,
            ApiError::Unauthorized | ApiError::NotFound | ApiError::ServerLocked => Self::Unavailable,
            ApiError::Problem(details) | ApiError::DirectMessageBlocked(details) => Self::Server {details},
            ApiError::Decode(_) => Self::Unavailable,
        }
    }
}
fn run<T>(future: impl std::future::Future<Output=Result<T, ApiError>>) -> Result<T, RecoveryError> {
    tokio::runtime::Builder::new_current_thread().enable_all().build()
        .map_err(|_| RecoveryError::Unavailable)?.block_on(future).map_err(Into::into)
}

/// Validation exposes only a hint. The caller must still select/confirm the
/// backend; neither a launch intent nor its query can redirect a reset request.
#[uniffi::export]
pub fn recovery_server(link: String) -> Result<String, RecoveryError> {
    Ok(recovery::parse(&link)?.server.unwrap_or_default())
}

#[uniffi::export]
pub fn recover_account(server: String, link: String, password: String) -> Result<(), RecoveryError> {
    let server = crate::normalize_url(&server).map_err(|_| RecoveryError::InvalidServer)?;
    let link = recovery::parse(&link)?;
    link.for_server(&server)?;
    if password.is_empty() { return Err(RecoveryError::EmptyPassword); }
    let api = Api::new(&server, Arc::new(Session::default()))?;
    run(api.recover_password(&link.token, &password))
}

#[uniffi::export]
impl NativeClient {
    pub fn password_reset_link(&self, owner: String, target: String) -> Result<RecoveryReceipt, RecoveryError> {
        // Check the owner-only capability before constructing any request. A
        // manage_server role cannot reset somebody else's account on this API.
        let server = {
            let backend = self.backend.lock().map_err(|_| RecoveryError::Unavailable)?;
            let Some(Backend::Live(c)) = backend.as_ref() else { return Err(RecoveryError::Unavailable); };
            let store = &c.runtime.store;
            if store.me != owner || !store.is_server_owner() { return Err(RecoveryError::NotOwner); }
            if target == owner || !store.members.iter().any(|m| m.id == target) { return Err(RecoveryError::InvalidMember); }
            c.runtime.url.clone()
        };
        let request = self.scoped_api(&owner)?;
        let response = run(request.api.reset_user(&target))?;
        // A late response must not expose a recovery token after an account
        // switch. Credentials and server address were frozen before HTTP began.
        self.finish_request(&request, vec![])?;
        let url = recovery::with_server(&response.reset_url, &server)?;
        Ok(RecoveryReceipt {url, expires_at: response.expires_at.to_rfc3339()})
    }
}
