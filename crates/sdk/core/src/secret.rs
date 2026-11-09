//! Asynchronous credential capability and generation-qualified keys; platform executors
//! own OS access, while this module owns deadlines, cancellation, and reconciliation.
//!
//! Storage must durably allocate generations that are never reused, even after account
//! removal/recreation. Platforms must map the entire [`SecretKey`] to distinct OS entries;
//! an old write/delete must never address a newer generation's entry. Only one boundary
//! may own a key at a time. Retired entries may need later cleanup: ignoring a stale
//! completion does not undo its OS effects or establish physical secret erasure.
//!
//! Dropping a platform future need not stop its OS operation. A read observing absence
//! is therefore insufficient to resolve an uncertain write. [`SecretRead::settled`]
//! promises a fence covering *all* previously submitted mutations for that exact key,
//! including work detached by cancellation, timeout, or a previous boundary instance.
//! If the platform cannot establish that fence, it must return an unfenced read (or
//! remain pending); the SDK keeps the key uncertain and cannot acknowledge removal.
//! A fresh generation can be used independently, but does not erase retired entries.

mod boundary;
#[cfg(test)]
mod generation_tests;
#[cfg(test)]
pub mod test_store;
#[cfg(test)]
mod tests;

use std::{future::Future, pin::Pin};

use crate::AccountId;

pub use boundary::SecretStoreBoundary;

/// An account's durable, never-reused credential epoch; Storage owns allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CredentialGeneration(u64);

impl CredentialGeneration {
    /// Wraps an epoch already allocated durably by Storage; does not allocate one.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the durable epoch number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Fully scoped OS entry identity; backends must not discard either component.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SecretKey {
    account: AccountId,
    generation: CredentialGeneration,
}

impl SecretKey {
    /// Qualifies an account's entry by its durable credential epoch.
    #[must_use]
    pub const fn new(account: AccountId, generation: CredentialGeneration) -> Self {
        Self {
            account,
            generation,
        }
    }

    /// Returns the owning account.
    #[must_use]
    pub const fn account(self) -> AccountId {
        self.account
    }

    /// Returns the credential epoch.
    #[must_use]
    pub const fn generation(self) -> CredentialGeneration {
        self.generation
    }
}

/// Opaque credential bytes, deliberately without `Debug`, `Display`, or serialization.
///
/// Exposure is explicit. Callers must never log or persist these bytes in SDK files.
/// This container does not promise memory zeroization.
pub struct SecretValue(Vec<u8>);

impl SecretValue {
    /// Takes ownership of credential bytes without formatting or validating them.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// Borrows bytes for authentication or transfer to a platform executor.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

/// A read observation and the platform's proof of mutation quiescence.
/// Deliberately does not implement `Debug` because it contains a secret.
pub struct SecretRead {
    value: Option<SecretValue>,
    settled: bool,
}

impl SecretRead {
    /// Reports a value after fencing every earlier mutation for this key.
    ///
    /// No such mutation may change the entry after this snapshot, even if its
    /// original future was dropped or belonged to an earlier boundary instance.
    #[must_use]
    pub fn settled(value: Option<SecretValue>) -> Self {
        Self {
            value,
            settled: true,
        }
    }

    /// Reports an observation without ruling out a still-running older mutation.
    /// This can never resolve an uncertain key, including when `value` is absent.
    #[must_use]
    pub fn unfenced(value: Option<SecretValue>) -> Self {
        Self {
            value,
            settled: false,
        }
    }
}

/// Object-safe asynchronous platform result; polling must never block a worker.
pub type SecretFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, SecretStoreError>> + Send + 'a>>;

/// Host credential access, independent of UI frameworks and SDK files.
///
/// Constructing and polling these futures must not block: hosts use a dedicated
/// executor for synchronous OS calls. OS work must not start before the future is
/// first polled. All mutation effects must finish before a
/// successful mutation result; errors may have unknown effects. Dropping a future
/// may leave work running, but it must remain restricted to the supplied key.
/// Reads must obey [`SecretRead`]'s fencing contract, including across restart.
pub trait SecretStore: Send + Sync {
    /// Reads the exact generation-qualified entry; absence is not an OS error.
    ///
    /// # Errors
    /// Returns a typed platform failure; no diagnostic may contain credential bytes.
    fn read(&self, key: SecretKey) -> SecretFuture<'_, SecretRead>;

    /// Replaces only this generation's entry; completion certifies all effects ended.
    ///
    /// # Errors
    /// A failure may have unknown effects and will require fenced read-back.
    fn write(&self, key: SecretKey, value: SecretValue) -> SecretFuture<'_, ()>;

    /// Idempotently removes only this generation's entry, including if already absent.
    ///
    /// # Errors
    /// A failure may have unknown effects and will require fenced read-back.
    fn delete(&self, key: SecretKey) -> SecretFuture<'_, ()>;
}

/// Platform failures are categories, never OS messages containing sensitive data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SecretStoreError {
    /// The platform store is temporarily unavailable.
    #[error("credential store unavailable")]
    Unavailable,
    /// The platform rejected access.
    #[error("credential store access denied")]
    AccessDenied,
    /// An operation failed; mutation effects might still have occurred.
    #[error("credential store operation failed")]
    Failed,
}

/// Whether a generation can be used without first reconciling an unknown mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CredentialState {
    /// No previously submitted mutation is known to have an unresolved outcome.
    Ready,
    /// A fenced read-back is mandatory before any subsequent use or mutation.
    Uncertain,
}

/// The cause of a credential operation's interruption or failed reconciliation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SecretFailure {
    /// The SDK's per-platform-call deadline expired.
    #[error("credential call timed out")]
    Timeout,
    /// The caller canceled the operation.
    #[error("credential call canceled")]
    Canceled,
    /// The platform reported a typed failure.
    #[error(transparent)]
    Store(SecretStoreError),
    /// A read could not fence potentially still-running mutations.
    #[error("credential mutation has no completion fence")]
    Unfenced,
}

/// SDK credential failures; no variant contains credential bytes or platform text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SecretError {
    /// A read failed without submitting a new mutation.
    #[error(transparent)]
    Failure(SecretFailure),
    /// Mutation outcome is unknown; never treat this as rollback or absence.
    #[error("credential outcome uncertain: {0}")]
    Uncertain(SecretFailure),
    /// A newer account generation replaced this call; ignore its completion.
    #[error("credential generation superseded")]
    Superseded,
    /// Another operation is already admitted for this account generation.
    #[error("credential generation busy")]
    Busy,
    /// Storage has not activated this account.
    #[error("credential account not activated")]
    UnknownAccount,
    /// Internal synchronization was poisoned; no OS operation was admitted.
    #[error("credential state unavailable")]
    Poisoned,
}
