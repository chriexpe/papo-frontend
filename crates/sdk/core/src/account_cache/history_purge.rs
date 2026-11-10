//! Resumable bounded history reclamation; durable visibility markers are published elsewhere.

use super::{
    AccountCache, CacheError,
    history_rows::{self, TABLES},
};
use crate::storage_worker::WorkerError;
use rusqlite::Transaction;

const TURN_ROWS: usize = 100;

impl AccountCache {
    /// Reclaim hidden incarnations in <=500-row shared-writer turns. A crash between
    /// turns leaves the durable manifest/revocation filter intact; the next open resumes.
    /// No generation claim is needed for maintenance, and active incarnations survive.
    pub(crate) async fn history_purge(&self) -> Result<(), CacheError> {
        let fence = self.fence().await?;
        let context = self.context();
        tokio::spawn(async move {
            let result = purge(&context).await;
            drop(fence);
            result
        })
        .await
        .map_err(|_| CacheError::TaskStopped)?
    }

    /// Before installing a newly opened cache handle, invalidate process-local epoch
    /// claims retained on disk, then resume interrupted reclamation. Identity is verified
    /// by the initializer before this function can access any history row.
    pub(super) async fn history_recover(&self) -> Result<(), CacheError> {
        loop {
            let account = self.account;
            let changed = self.writer.cache_write(move |transaction| {
                history_rows::verify(transaction, account)?;
                transaction.execute("UPDATE history_channels SET verified_generation=NULL WHERE channel_id IN (SELECT channel_id FROM history_channels WHERE verified_generation IS NOT NULL LIMIT 100)", []).map_err(WorkerError::from).map_err(CacheError::from)
            }).await?;
            if changed < 500 {
                break;
            }
        }
        self.history_purge().await
    }
}

pub(super) async fn purge(context: &super::CacheContext) -> Result<(), CacheError> {
    loop {
        let account = context.account;
        let changed = context
            .writer
            .cache_write(move |transaction| {
                history_rows::verify(transaction, account)?;
                turn(transaction)
            })
            .await?;
        if changed == 0 {
            return Ok(());
        }
    }
}

/// One atomic maintenance turn. Explicit row budgets avoid foreign-key cascades and
/// ensure a large old set cannot monopolize the shared writer across accounts.
pub(super) fn turn(transaction: &Transaction<'_>) -> Result<usize, CacheError> {
    let mut changed = transaction.execute(
        "UPDATE history_sets SET revoked=1 WHERE revoked=0 AND id IN (
            SELECT h.current_set FROM history_channels h WHERE h.current_set IS NOT NULL
              AND NOT EXISTS(SELECT 1 FROM history_manifest m WHERE m.channel_id=h.channel_id
                AND m.set_id=h.current_set AND m.revision=(SELECT history_revision FROM cache_metadata WHERE singleton=1))
            LIMIT 100)", []
    ).map_err(WorkerError::from)?;
    if changed == TURN_ROWS {
        return Ok(changed);
    }
    changed += transaction
        .execute(
            "UPDATE history_channels SET current_set=NULL WHERE channel_id IN (
            SELECT h.channel_id FROM history_channels h JOIN history_sets s ON s.id=h.current_set
            WHERE s.revoked=1 LIMIT ?1)",
            [limit(changed)?],
        )
        .map_err(WorkerError::from)?;
    if changed == TURN_ROWS {
        return Ok(changed);
    }
    changed += transaction
        .execute(
            "DELETE FROM history_manifest WHERE rowid IN (SELECT m.rowid FROM history_manifest m
            JOIN history_sets s ON s.id=m.set_id WHERE s.revoked=1
                OR m.revision<>(SELECT history_revision FROM cache_metadata WHERE singleton=1)
            LIMIT ?1)",
            [limit(changed)?],
        )
        .map_err(WorkerError::from)?;
    for table in TABLES {
        if changed == TURN_ROWS {
            return Ok(changed);
        }
        changed += table.purge(transaction, TURN_ROWS.saturating_sub(changed))?;
    }
    if changed == TURN_ROWS {
        return Ok(changed);
    }
    changed += transaction.execute(
        "DELETE FROM history_sets WHERE id IN (SELECT s.id FROM history_sets s WHERE s.revoked=1
            AND NOT EXISTS(SELECT 1 FROM history_channels h WHERE h.current_set=s.id)
            AND NOT EXISTS(SELECT 1 FROM history_manifest m WHERE m.set_id=s.id)
            AND NOT EXISTS(SELECT 1 FROM chunks c WHERE c.set_id=s.id)
            AND NOT EXISTS(SELECT 1 FROM chunk_items i WHERE i.set_id=s.id)
            AND NOT EXISTS(SELECT 1 FROM messages m WHERE m.set_id=s.id)
            AND NOT EXISTS(SELECT 1 FROM pins p WHERE p.set_id=s.id)
            LIMIT ?1)", [limit(changed)?]
    ).map_err(WorkerError::from)?;
    Ok(changed)
}

fn limit(changed_rows: usize) -> Result<i64, CacheError> {
    i64::try_from(TURN_ROWS.saturating_sub(changed_rows)).map_err(|_| CacheError::InvalidData)
}
