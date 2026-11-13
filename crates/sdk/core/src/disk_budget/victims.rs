//! Global least-recently-opened channel selection; media eviction and history mutation live separately.

use super::DiskError;
use crate::{AccountCache, AccountId, ClientError, ClientHandle, Uuid};
use std::sync::Arc;

pub(super) type Candidate = (i64, AccountId, Uuid, Arc<AccountCache>);

pub(super) async fn select(
    client: &ClientHandle,
) -> Result<(Vec<Candidate>, Vec<Arc<AccountCache>>), DiskError> {
    let mut caches = Vec::new();
    let mut candidates = Vec::new();
    for server in client
        .inner
        .registry
        .servers()
        .await
        .map_err(ClientError::from)?
    {
        for account in client
            .inner
            .registry
            .accounts(server.address.id())
            .await
            .map_err(ClientError::from)?
        {
            let _lifecycle = client.inner.pool.lifecycle(account.id).read_owned().await;
            if !client
                .inner
                .registry
                .accounts(account.id.server())
                .await
                .map_err(ClientError::from)?
                .iter()
                .any(|current| current.id == account.id)
            {
                continue;
            }
            if !client.inner.media.root.account_cache(account.id).exists() {
                continue;
            }
            let cache = client
                .inner
                .caches
                .open(account.id)
                .await
                .map_err(ClientError::from)?;
            caches.push(Arc::clone(&cache));
            for id in cache
                .retention_channels()
                .await
                .map_err(ClientError::from)?
            {
                let channel = crate::Uuid::from_bytes(id);
                let opened = client.inner.media.last_opened(account.id, channel).await?;
                candidates.push((opened, account.id, channel, Arc::clone(&cache)));
            }
        }
    }
    candidates.sort_by_key(|(opened, account, channel, _)| (*opened, *account, *channel));
    Ok((candidates, caches))
}
