//! Opaque incremental history rows and bounded keyset reads; protocol meaning belongs to the model.

use super::{CacheError, history, schema};
use crate::{Generation, storage_worker::WorkerError};
use rusqlite::{Transaction, params};

/// Persisted model partitions; payloads are opaque to storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HistoryTable {
    Chunks,
    Items,
    Messages,
    Pins,
}

/// One incremental model payload, scoped to a history incarnation by the caller.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HistoryRow {
    pub table: HistoryTable,
    pub id: [u8; 16],
    pub payload: Vec<u8>,
}

/// One incremental deletion; storage never infers protocol deletion semantics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HistoryKey {
    pub table: HistoryTable,
    pub id: [u8; 16],
}

/// A coherent model incarnation, including metadata for an empty history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HistoryData {
    pub set_id: i64,
    pub metadata: Vec<u8>,
    pub rows: Vec<HistoryRow>,
}

pub(super) const TABLES: [HistoryTable; 4] = [
    HistoryTable::Chunks,
    HistoryTable::Items,
    HistoryTable::Messages,
    HistoryTable::Pins,
];

impl HistoryTable {
    fn name(self) -> &'static str {
        match self {
            Self::Chunks => "chunks",
            Self::Items => "chunk_items",
            Self::Messages => "messages",
            Self::Pins => "pins",
        }
    }

    pub(super) fn upsert(
        self,
        transaction: &Transaction<'_>,
        set_id: i64,
        row: &HistoryRow,
    ) -> Result<(), CacheError> {
        transaction.execute(
            &format!("INSERT INTO {} (set_id,id,payload) VALUES (?1,?2,?3) ON CONFLICT(set_id,id) DO UPDATE SET payload=excluded.payload", self.name()),
            params![set_id, row.id.as_slice(), row.payload],
        ).map_err(WorkerError::from)?;
        Ok(())
    }

    pub(super) fn delete(
        self,
        transaction: &Transaction<'_>,
        set_id: i64,
        id: [u8; 16],
    ) -> Result<(), CacheError> {
        transaction
            .execute(
                &format!("DELETE FROM {} WHERE set_id=?1 AND id=?2", self.name()),
                params![set_id, id.as_slice()],
            )
            .map_err(WorkerError::from)?;
        Ok(())
    }

    pub(super) fn purge(
        self,
        transaction: &Transaction<'_>,
        limit_rows: usize,
    ) -> Result<usize, CacheError> {
        transaction.execute(
            &format!("DELETE FROM {} WHERE rowid IN (SELECT rowid FROM {} WHERE set_id IN (SELECT id FROM history_sets WHERE revoked=1) LIMIT ?1)", self.name(), self.name()),
            [i64::try_from(limit_rows).map_err(|_| CacheError::InvalidData)?],
        ).map_err(WorkerError::from).map_err(CacheError::from)
    }

    pub(super) fn read(
        self,
        transaction: &Transaction<'_>,
        channel: [u8; 16],
        generation: Generation,
        set_id: i64,
        after: Option<[u8; 16]>,
    ) -> Result<Vec<HistoryRow>, CacheError> {
        // Rechecking the marker in every batch prevents a future reader implementation
        // from accidentally returning partially purged rows without a live incarnation.
        if history::authorized(transaction, channel, generation)? != Some(set_id) {
            return Err(CacheError::InvalidData);
        }
        let mut statement = transaction.prepare(&format!(
            "SELECT id,payload FROM {} WHERE set_id=?1 AND (?2 IS NULL OR id>?2) ORDER BY id LIMIT 500", self.name()
        )).map_err(WorkerError::from)?;
        let rows = statement
            .query_map(params![set_id, after.map(|id| id.to_vec())], |row| {
                let id: Vec<u8> = row.get(0)?;
                let id = id.try_into().map_err(|_| rusqlite::Error::InvalidQuery)?;
                Ok(HistoryRow {
                    table: self,
                    id,
                    payload: row.get(1)?,
                })
            })
            .map_err(WorkerError::from)?;
        rows.map(|row| row.map_err(WorkerError::from).map_err(CacheError::from))
            .collect()
    }
}

/// Check account metadata before a history-only query made without `CacheContext::query`.
pub(super) fn verify(
    transaction: &Transaction<'_>,
    account: crate::AccountId,
) -> Result<(), CacheError> {
    schema::metadata(transaction, account).map_err(WorkerError::from)?;
    Ok(())
}
