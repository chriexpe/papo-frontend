//! Persisted non-secret registry records; live auth and server probing belong to services.

use crate::secret::CredentialGeneration;
use crate::{AccountId, ServerAddress};

/// Microseconds since the Unix epoch, supplied by the caller's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnixMicros(pub i64);

/// A visible saved server in durable list order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedServer {
    /// Validated canonical address; Debug remains redacted.
    pub address: ServerAddress,
    /// Time the entry was registered, in Unix microseconds.
    pub added_at: UnixMicros,
}

/// Device-local account metadata, never a proof of authentication.
#[derive(Clone, PartialEq, Eq)]
pub struct SavedAccount {
    /// Server plus user identity.
    pub id: AccountId,
    /// Last known non-secret display name.
    pub display_name: Option<String>,
    /// Last known non-secret media reference.
    pub avatar_ref: Option<String>,
    /// Exactly zero or one visible account per server is active.
    pub active: bool,
    /// Registration time, in Unix microseconds.
    pub created_at: UnixMicros,
    /// Never-reused generation for the OS credential key.
    pub credential_generation: CredentialGeneration,
}

impl std::fmt::Debug for SavedAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SavedAccount")
            .field("id", &self.id)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}
