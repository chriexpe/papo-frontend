//! Bounded history eviction and physical SQLite reclamation; global policy and measurement live above.

use super::{DiskError, DiskReport, measure, measurement, victims};
use crate::{ClientError, ClientHandle, MediaError, Session};
use std::sync::Arc;

pub(super) struct HistoryTrim {
    pub turns: u64,
    pub max_rows: usize,
    pub blocked: bool,
}
pub(super) async fn history(
    client: &ClientHandle,
    candidates: Vec<victims::Candidate>,
    mut report: DiskReport,
) -> Result<HistoryTrim, DiskError> {
    let mut history_turns = 0;
    let mut max_history_rows_per_turn = 0;
    let mut reclaim_blocked = false;
    for (_, account, channel, cache) in candidates {
        let mut retainer = crate::history::retention::Retainer::new(Arc::clone(&cache), channel);
        while report.pressure {
            if client.is_closed() {
                return Err(ClientError::Closed.into());
            }
            let lifecycle = client.inner.pool.lifecycle(account).read_owned().await;
            let present = client
                .inner
                .registry
                .accounts(account.server())
                .await
                .map_err(ClientError::from)?
                .iter()
                .any(|entry| entry.id == account);
            if !present {
                break;
            }
            let before_database = database_usage(client, account).await?;
            let session = client.inner.pool.get(account).map(|inner| Session {
                inner,
                client: client.clone(),
            });
            let Some(rows) = retainer.evict(session).await? else {
                break;
            };
            max_history_rows_per_turn = max_history_rows_per_turn.max(rows);
            history_turns += 1;
            let busy = cache.reclaim().await.map_err(ClientError::from)?;
            reclaim_blocked |= busy;
            let after_database = database_usage(client, account).await?;
            drop(lifecycle);
            if history_turns % 64 == 0 {
                report = measure(client).await?;
            } else {
                let Some(database_bytes) = report
                    .database_bytes
                    .checked_sub(before_database)
                    .and_then(|n| n.checked_add(after_database))
                else {
                    report = measure(client).await?;
                    continue;
                };
                report.database_bytes = database_bytes;
                report.allocated_bytes = report
                    .database_bytes
                    .checked_add(report.media_bytes)
                    .and_then(|n| n.checked_add(report.outgoing_bytes))
                    .and_then(|n| n.checked_add(report.profile_bytes))
                    .ok_or(DiskError::Measurement)?;
                report.pressure = report.allocated_bytes > report.budget_bytes;
            }
            if busy {
                break;
            }
        }
        if !report.pressure {
            break;
        }
    }
    Ok(HistoryTrim {
        turns: history_turns,
        max_rows: max_history_rows_per_turn,
        blocked: reclaim_blocked,
    })
}

async fn database_usage(
    client: &ClientHandle,
    account: crate::AccountId,
) -> Result<u64, DiskError> {
    let path = client.inner.media.root.account_cache(account);
    tokio::task::spawn_blocking(move || measurement::database(&path))
        .await
        .map_err(|_| DiskError::Measurement)?
}

pub(super) async fn existing(
    client: &ClientHandle,
    caches: &[Arc<crate::AccountCache>],
) -> Result<bool, DiskError> {
    let mut blocked = client
        .inner
        .registry
        .writer
        .truncate_wal()
        .await
        .map_err(|error| ClientError::Storage(error.into()))?;
    loop {
        if client.is_closed() {
            return Err(ClientError::Closed.into());
        }
        let path = client.inner.media.root.media_index();
        let previous = tokio::task::spawn_blocking(move || measurement::database(&path))
            .await
            .map_err(|_| DiskError::Measurement)??;
        let busy = client
            .inner
            .media
            .writer
            .reclaim()
            .await
            .map_err(MediaError::from)?;
        blocked |= busy;
        let path = client.inner.media.root.media_index();
        let current = tokio::task::spawn_blocking(move || measurement::database(&path))
            .await
            .map_err(|_| DiskError::Measurement)??;
        if busy || current >= previous {
            break;
        }
    }
    for cache in caches {
        let account = cache.account_id();
        loop {
            if client.is_closed() {
                return Err(ClientError::Closed.into());
            }
            let _lifecycle = client.inner.pool.lifecycle(account).read_owned().await;
            if !client
                .inner
                .registry
                .accounts(account.server())
                .await
                .map_err(ClientError::from)?
                .iter()
                .any(|entry| entry.id == account)
            {
                break;
            }
            let previous = database_usage(client, account).await?;
            let busy = cache.reclaim().await.map_err(ClientError::from)?;
            blocked |= busy;
            if busy || database_usage(client, account).await? >= previous {
                break;
            }
        }
    }
    Ok(blocked)
}
