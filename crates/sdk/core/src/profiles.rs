//! On-demand generation-stamped profile batches; member listing and live invalidation belong to reconcile.

use std::collections::HashSet;

use papo_api::{
    Uuid,
    wire::{ProfileBatchRequest, UserProfile},
};

use crate::{
    ClientError, Generated, Session,
    generation::Epoch,
    history::access::Refetch,
    reconcile::ResourceConnection,
    resource_admission::Admission,
    resource_commit::{self, FetchResult, FetchStamp},
    resources::AdmittedMutation,
};

impl Session {
    /// Fetches up to 1000 requested IDs in batches of at most 50, deduplicating IDs.
    /// Results and cache publication are fenced by the requesting connection generation.
    /// Shared snapshots retain metadata only. Inline avatar blobs stay in the returned owned response.
    /// # Errors
    /// Returns revoked/stale generations, identity, transport or rebuildable storage failures.
    pub async fn profiles(
        &self,
        users: &[Uuid],
    ) -> Result<Generated<Vec<UserProfile>>, ClientError> {
        if users.len() > 1000 {
            return Err(ClientError::ProfileLimit);
        }
        let mut seen = HashSet::new();
        let users = users
            .iter()
            .filter(|user| seen.insert(**user))
            .copied()
            .collect();
        let session = self.clone();
        let token = self.inner.tasks.token();
        self.client
            .dispatch(async move {
                let _token = token;
                let epoch = session.inner.epochs.current();
                session.inner.epochs.validate(epoch.generation)?;
                tokio::select! {
                    biased;
                    () = session.inner.root.cancelled() => Err(ClientError::Closed),
                    () = epoch.cancel.cancelled() => Err(ClientError::Superseded),
                    result = session.fetch_profiles(&epoch, users) => result,
                }
            })
            .await
    }

    async fn fetch_profiles(
        &self,
        epoch: &Epoch,
        users: Vec<Uuid>,
    ) -> Result<Generated<Vec<UserProfile>>, ClientError> {
        let secret = self
            .inner
            .read_secret(&self.client.inner.secrets, &epoch.cancel)
            .await?
            .ok_or(ClientError::SignedOut)?;
        let rest = self
            .client
            .inner
            .transport(&self.inner.address, Some(secret.expose()))?;
        let who = self.client.inner.request(rest.whoami()).await?;
        self.inner.epochs.validate(epoch.generation)?;
        if who.id.as_deref().and_then(|id| id.parse().ok()) != Some(self.account().user()) {
            return Err(ClientError::Identity);
        }
        let cache = self.client.inner.caches.open(self.account()).await?;
        let connection = ResourceConnection {
            client: self.client.clone(),
            rest,
            cache,
        };
        let mut profiles = Vec::new();
        for ids in users.chunks(50) {
            let guard = resource_commit::reserve(&self.inner).await;
            let stamp =
                FetchStamp::capture(&self.inner, epoch.generation, Refetch::Partial).await?;
            let request = ProfileBatchRequest {
                ids: ids.iter().map(ToString::to_string).collect(),
            };
            let response = self
                .client
                .inner
                .request(connection.rest.profiles(&request))
                .await;
            let fetched: FetchResult = (|| {
                let mut batch = response?.profiles;
                let metadata = batch.as_mut().map(|profiles| {
                    profiles
                        .iter_mut()
                        .map(|profile| {
                            let avatar = profile.avatar_blob.take();
                            let retained = profile.clone();
                            profile.avatar_blob = avatar;
                            retained
                        })
                        .collect()
                });
                let batch_facts = Admission::new(&self.inner).profiles(ids, metadata)?;
                profiles.extend(batch.into_iter().flatten());
                Ok(AdmittedMutation {
                    profiles: vec![batch_facts],
                    ..Default::default()
                })
            })();
            resource_commit::commit(&self.inner, &connection, guard, stamp, fetched)
                .await?
                .require_current()?;
        }
        Ok(Generated {
            generation: epoch.generation,
            value: profiles,
        })
    }
}
