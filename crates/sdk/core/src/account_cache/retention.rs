//! Maintenance-only opaque history reads; reading these rows never creates session authority.

use super::{
    AccountCache, CacheError,
    history::HistoryData,
    history_rows::{HistoryRow, TABLES},
};
use crate::storage_worker::WorkerError;
use rusqlite::{OptionalExtension, params};
use std::sync::Arc;

impl AccountCache {
    pub(crate) fn retention_clock(&self, channel: [u8; 16]) -> Arc<std::sync::atomic::AtomicU64> {
        Arc::clone(
            self.retention
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .entry(channel)
                .or_default(),
        )
    }
    pub(crate) fn retention_version(&self, channel: [u8; 16]) -> u64 {
        self.retention_clock(channel)
            .load(std::sync::atomic::Ordering::Acquire)
    }
    pub(crate) async fn reclaim(&self) -> Result<bool, CacheError> {
        let _fence = self.fence().await?;
        Ok(self.writer.reclaim().await?)
    }
    pub(crate) async fn retention_commit(
        &self,
        channel: [u8; 16],
        set_id: i64,
        expected_revision: u64,
        metadata: Vec<u8>,
        rows: Vec<HistoryRow>,
        deletes: Vec<super::history::HistoryKey>,
    ) -> Result<(), CacheError> {
        if rows.len().saturating_add(deletes.len()) > 499 {
            return Err(WorkerError::RowLimit.into());
        }
        let fence = self.fence().await?;
        let context = self.context();
        let retention = self.retention_clock(channel);
        tokio::spawn(async move {
            let account = context.account;
            let result = context
                .writer
                .cache_write(move |tx| {
                    super::history_rows::verify(tx, account)?;
                    let current: Option<i64> = tx
                        .query_row(
                            "SELECT current_set FROM history_channels h
                     JOIN history_sets s ON s.id=h.current_set
                     WHERE h.channel_id=?1 AND s.revoked=0",
                            [channel.as_slice()],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(WorkerError::from)?;
                    if current != Some(set_id) {
                        return Err(CacheError::Superseded);
                    }
                    let revision: i64 = tx.query_row(
                    "SELECT json_extract(metadata,'$.revision') FROM history_sets WHERE id=?1",
                    [set_id],
                    |row| row.get(0),
                ).map_err(WorkerError::from)?;
                    let revision = u64::try_from(revision).map_err(|_| CacheError::InvalidData)?;
                    if revision != expected_revision {
                        return Err(CacheError::Superseded);
                    }
                    for key in deletes {
                        key.table.delete(tx, set_id, key.id)?;
                    }
                    for row in rows {
                        row.table.upsert(tx, set_id, &row)?;
                    }
                    tx.execute(
                        "UPDATE history_sets SET metadata=?1 WHERE id=?2",
                        params![metadata, set_id],
                    )
                    .map_err(WorkerError::from)?;
                    Ok(())
                })
                .await;
            if result.is_ok() {
                retention.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            }
            drop(fence);
            result
        })
        .await
        .map_err(|_| CacheError::TaskStopped)?
    }
    pub(crate) async fn retention_channels(&self) -> Result<Vec<[u8; 16]>, CacheError> {
        let fence = self.fence().await?;
        let context = self.context();
        let (_, ids) = context
            .query(fence, |tx| {
                let mut statement = tx
                    .prepare(
                        "SELECT h.channel_id
                     FROM history_channels h
                     JOIN history_sets s ON s.id=h.current_set
                     WHERE s.revoked=0
                     AND EXISTS(SELECT 1
                     FROM messages m
                     WHERE m.set_id=s.id)
                     ORDER BY h.channel_id",
                    )
                    .map_err(WorkerError::from)?;
                let rows = statement
                    .query_map([], |row| row.get::<_, Vec<u8>>(0))
                    .map_err(WorkerError::from)?;
                rows.map(|row| {
                    row.map_err(WorkerError::from)?
                        .try_into()
                        .map_err(|_| CacheError::InvalidData)
                })
                .collect()
            })
            .await?;
        Ok(ids)
    }

    pub(crate) async fn retention_data(
        &self,
        channel: [u8; 16],
    ) -> Result<Option<HistoryData>, CacheError> {
        let fence = self.fence().await?;
        let context = self.context();
        let (mut fence, initial) = context
            .query(fence, move |tx| {
                tx.query_row(
                    "SELECT s.id,s.metadata
                     FROM history_channels h
                     JOIN history_sets s ON s.id=h.current_set
                     WHERE h.channel_id=?1
                     AND s.revoked=0",
                    [channel.as_slice()],
                    |row| {
                        Ok(HistoryData {
                            set_id: row.get(0)?,
                            metadata: row.get(1)?,
                            rows: Vec::new(),
                        })
                    },
                )
                .optional()
                .map_err(WorkerError::from)
                .map_err(CacheError::from)
            })
            .await?;
        let Some(mut data) = initial else {
            return Ok(None);
        };
        for table in TABLES {
            let mut after = Vec::new();
            loop {
                let set_id = data.set_id;
                let (returned, rows) = context.query(fence, move |tx| {
                    let sql = format!(
                        "SELECT id,payload FROM {} WHERE set_id=?1 AND id>?2 ORDER BY id LIMIT 100",
                        table.name(),
                    );
                    let mut statement = tx.prepare(&sql).map_err(WorkerError::from)?;
                    let rows = statement.query_map(params![set_id, after], |row| {
                        let id: Vec<u8> = row.get(0)?;
                        Ok(HistoryRow {
                            table,
                            id: id.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?,
                            payload: row.get(1)?,
                        })
                    }).map_err(WorkerError::from)?;
                    rows.map(|row| row.map_err(WorkerError::from).map_err(CacheError::from))
                        .collect::<Result<Vec<_>, _>>()
                }).await?;
                fence = returned;
                let count = rows.len();
                after = rows.last().map_or_else(Vec::new, |row| row.id.to_vec());
                data.rows.extend(rows);
                if count < 100 {
                    break;
                }
            }
        }
        Ok(Some(data))
    }
}
