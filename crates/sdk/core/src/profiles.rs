//! On-demand generation-stamped profile batches; member listing and live invalidation belong to reconcile.

use std::sync::Arc;

use papo_api::{
    Uuid,
    wire::{ProfileBatchRequest, UserProfile},
};

use crate::{CacheResource, ClientError, Generated, Session, generation::Epoch};

impl Session {
    /// Fetches profiles on demand in backend-sized batches of at most 50.
    /// Results and cache publication are fenced by the requesting connection generation.
    /// # Errors
    /// Returns revoked/stale generations, identity, transport or rebuildable storage failures.
    pub async fn profiles(
        &self,
        users: &[Uuid],
    ) -> Result<Generated<Vec<UserProfile>>, ClientError> {
        let users = users.iter().map(ToString::to_string).collect();
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
        users: Vec<String>,
    ) -> Result<Generated<Vec<UserProfile>>, ClientError> {
        let _serial = self.inner.resource_serial.lock().await;
        let secret = self
            .client
            .inner
            .secrets
            .read(self.inner.key, &epoch.cancel)
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
        let mut profiles = Vec::new();
        for ids in users.chunks(50) {
            let request = ProfileBatchRequest { ids: ids.to_vec() };
            let response = self.client.inner.request(rest.profiles(&request)).await?;
            profiles.extend(response.profiles.unwrap_or_default());
        }
        self.inner.epochs.validate(epoch.generation)?;
        let mut snapshot = (**self.inner.resources.borrow()).clone();
        if snapshot.generation != epoch.generation {
            return Err(ClientError::Superseded);
        }
        let cached = Arc::make_mut(&mut snapshot.profiles);
        for profile in &profiles {
            let id = profile.id.as_ref().ok_or(ClientError::ResourceData)?;
            if !users.contains(id) {
                return Err(ClientError::ResourceData);
            }
            cached.retain(|existing| existing.id.as_ref() != Some(id));
            cached.push(profile.clone());
        }
        cache.update(&snapshot, &[CacheResource::Profiles]).await?;
        self.inner.publish_resources(snapshot)?;
        Ok(Generated {
            generation: epoch.generation,
            value: profiles,
        })
    }
}
