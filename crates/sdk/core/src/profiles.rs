//! On-demand generation-stamped profile batches; member listing and live invalidation belong to reconcile.

use std::{collections::HashSet, sync::Arc};

use papo_api::{
    Uuid,
    wire::{ProfileBatchRequest, UserProfile},
};

use crate::{CacheResource, ClientError, Generated, Session, generation::Epoch};

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
            .map(ToString::to_string)
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
        users: Vec<String>,
    ) -> Result<Generated<Vec<UserProfile>>, ClientError> {
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
            // Serialize one bounded batch with member invalidation, then yield to waiting live merges.
            let _serial = self.inner.resource_serial.lock().await;
            self.inner.epochs.validate(epoch.generation)?;
            let request = ProfileBatchRequest { ids: ids.to_vec() };
            let response = self.client.inner.request(rest.profiles(&request)).await?;
            let mut batch = response.profiles.unwrap_or_default();
            let mut returned = HashSet::new();
            for profile in &batch {
                let id = profile.id.as_ref().ok_or(ClientError::ResourceData)?;
                if !ids.contains(id) || !returned.insert(id) {
                    return Err(ClientError::ResourceData);
                }
            }
            self.inner.epochs.validate(epoch.generation)?;
            let mut snapshot = (**self.inner.resources.borrow()).clone();
            if snapshot.generation != epoch.generation {
                return Err(ClientError::Superseded);
            }
            let cached = Arc::make_mut(&mut snapshot.profiles);
            cached.retain(|existing| !existing.id.as_ref().is_some_and(|id| ids.contains(id)));
            for profile in &mut batch {
                // Do not clone the base64 bytes into the shared resource image.
                let avatar = profile.avatar_blob.take();
                cached.push(profile.clone());
                profile.avatar_blob = avatar;
            }
            cache.update(&snapshot, &[CacheResource::Profiles]).await?;
            self.inner.publish_resources(snapshot)?;
            profiles.extend(batch);
        }
        Ok(Generated {
            generation: epoch.generation,
            value: profiles,
        })
    }
}
