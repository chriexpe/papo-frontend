//! Account-scoped acknowledged operations; domain modules own inputs and backend semantics.
//! Ambiguous dispatch is never retried. Rebuildable refresh failure cannot undo server success.

pub mod administration;
pub mod audit;
pub mod channels;
pub mod emojis;
mod execution;
pub mod interactions;
mod metadata;
pub use metadata::MetadataScope;
pub mod moderation;
pub mod profile;
pub mod reactions;
mod refresh;
mod role_projection;
pub mod roles;
pub mod settings;
pub mod social;
pub mod webhooks;

use crate::{AccountId, ClientError, Generated, Generation, Session};
use refresh::Refresh;

/// Typed operation failure, with uncertain mutations distinguished from definite rejection.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OperationError {
    /// Input cannot be represented by the actual backend contract.
    #[error("invalid operation input")]
    InvalidInput,
    /// Backend response lacks required identity or pagination evidence.
    #[error("invalid operation response")]
    InvalidData,
    /// Requested resource is absent from the authoritative response.
    #[error("operation resource not found")]
    NotFound,
    /// Dispatch may have committed. Reconcile or ask explicitly; never automatically retry.
    #[error("operation outcome unknown: {0}")]
    Unknown(ClientError),
    /// Pre-dispatch lifecycle failure or definite backend rejection.
    #[error(transparent)]
    Session(#[from] ClientError),
}

/// Independent refresh warning after a positively acknowledged server mutation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefreshFault {
    /// A newer resource publication arrived during the fetch; it was preserved.
    ConcurrentChange,
    /// Backend acknowledged a setting that its read endpoints cannot expose.
    Unavailable,
    /// Account removal or logout ended the result's capability.
    Closed,
    /// Reconnect or replacement ended the requesting generation.
    Superseded,
    /// Rebuildable cache failed; mutation remains acknowledged.
    Cache,
    /// Refresh transport, authorization or response validation failed.
    Request,
}
/// Whether acknowledged server state has been refreshed into local projections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RefreshStatus {
    /// This operation has no retained SDK projection to refresh.
    NotRequired,
    /// Local metadata/history accepted the authoritative refresh.
    Updated,
    /// Server mutation succeeded; the independent local refresh did not.
    Warning(RefreshFault),
}
/// Positive server acknowledgment, never an optimistic mutation or a retry instruction.
#[derive(Clone)]
pub struct OperationReceipt<T> {
    /// Source account; never inferred from current UI selection.
    pub account: AccountId,
    /// Generation that dispatched the operation.
    pub generation: Generation,
    /// Acknowledged response. Secrets in explicit credential responses remain caller-owned.
    pub value: T,
    /// Failure here does not change the successful server outcome.
    pub refresh: RefreshStatus,
}
impl<T> OperationReceipt<T> {
    /// Validates the original capability before presentation or navigation.
    /// # Errors
    /// Returns identity mismatch, closed or superseded without exposing a stale value.
    pub fn validate(&self, session: &Session) -> Result<(), OperationError> {
        if self.account != session.account() {
            return Err(ClientError::Identity.into());
        }
        session.inner.epochs.validate(self.generation)?;
        Ok(())
    }
}

/// Typed account operations. Clones share mutation ordering, not another send queue or runtime.
/// Backend authorization remains mandatory even when UI capabilities offer an action.
#[derive(Clone)]
pub struct Operations {
    pub(super) session: Session,
    expected_generation: Option<Generation>,
}
impl Session {
    /// Returns typed administrative and personal operations for this revocable account.
    #[must_use]
    pub fn operations(&self) -> Operations {
        Operations::new(self.clone())
    }
}
impl Operations {
    /// Binds operations to one account capability; replacing the account revokes every clone.
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self {
            session,
            expected_generation: None,
        }
    }
    fn for_batch(&self) -> Self {
        Self {
            session: self.session.clone(),
            expected_generation: Some(
                self.expected_generation
                    .unwrap_or_else(|| self.session.generation()),
            ),
        }
    }
    /// Returns the source account for operation results and explicit confirmations.
    #[must_use]
    pub fn account(&self) -> AccountId {
        self.session.account()
    }
}
