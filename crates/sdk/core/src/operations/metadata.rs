//! Explicit refresh-only recovery after an accepted operation; this never repeats a mutation.

use super::{OperationError, Operations, Refresh, RefreshStatus};
use crate::{Generated, Uuid};

/// Bounded metadata domain to refresh independently of a prior mutation acknowledgment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MetadataScope {
    /// Server metadata, excluding inline icon bytes.
    Server,
    /// Current backend channel list and overrides.
    Channels,
    /// Role definitions and labels of already-retained people.
    Roles,
    /// Current account DM visibility and outbound blocks.
    Social,
    /// Backend account notification policy, preserving independent device overrides.
    AccountSettings,
    /// One explicitly requested profile and its lightweight member summary.
    Profile(Uuid),
    /// One member summary without requesting avatar/profile data.
    Member(Uuid),
    /// Canonical pin snapshot, without read acknowledgment or timeline coverage.
    Pins(Uuid),
}
impl From<MetadataScope> for Refresh {
    fn from(scope: MetadataScope) -> Self {
        match scope {
            MetadataScope::Server => Self::Server,
            MetadataScope::Channels => Self::Channels,
            MetadataScope::Roles => Self::Roles,
            MetadataScope::Social => Self::Social,
            MetadataScope::AccountSettings => Self::Policy,
            MetadataScope::Profile(user) => Self::Profile(user),
            MetadataScope::Member(user) => Self::Member(user),
            MetadataScope::Pins(channel) => Self::Pins(channel),
        }
    }
}
impl Operations {
    /// Retries only the selected metadata read/publication after a refresh warning.
    /// No mutation request is made, and no ambiguous operation is retried.
    /// # Errors
    /// Returns account revocation, generation replacement or authentication failure.
    /// Fetch/cache/concurrent-publication faults are typed refresh warnings.
    pub async fn refresh_metadata(
        &self,
        scope: MetadataScope,
    ) -> Result<Generated<RefreshStatus>, OperationError> {
        let session = self.session.clone();
        let epoch = session.inner.epochs.current();
        if self
            .expected_generation
            .is_some_and(|expected| expected != epoch.generation)
        {
            return Err(crate::ClientError::Superseded.into());
        }
        let token = session.inner.tasks.token();
        Ok(self
            .session
            .client
            .dispatch(async move {
                let _token = token;
                let work = async {
                    let _serial = session.inner.operations_serial.lock().await;
                    session.inner.epochs.validate(epoch.generation)?;
                    let rest = crate::scoped_request::authenticated(&session, &epoch).await?;
                    let value =
                        match super::refresh::apply(&session, &epoch, &rest, scope.into()).await {
                            Ok(()) => RefreshStatus::Updated,
                            Err(fault) => RefreshStatus::Warning(fault),
                        };
                    session.inner.epochs.validate(epoch.generation)?;
                    Ok(Generated {
                        generation: epoch.generation,
                        value,
                    })
                };
                tokio::select! {
                    biased;
                    () = session.inner.root.cancelled() => Err(crate::ClientError::Closed),
                    () = epoch.cancel.cancelled() => Err(crate::ClientError::Superseded),
                    result = work => result,
                }
            })
            .await?)
    }
}
