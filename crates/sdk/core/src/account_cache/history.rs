//! Fenced history storage admission and incarnation authorization; model semantics live elsewhere.

pub(crate) use super::history_rows::{HistoryData, HistoryKey, HistoryRow, HistoryTable};
use super::{AccountCache, CacheError, history_rows::TABLES};
use crate::{Generation, storage_worker::WorkerError};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use tokio_util::sync::CancellationToken;

impl AccountCache {
    /// Worker-unique database identity; numeric history-set IDs are only unique within this instance.
    pub(crate) fn history_instance(&self) -> u64 {
        self.writer.database_id()
    }

    /// Admit a short cache callback under the shared serial fence. Generation is claimed
    /// before admission and revalidated on the actual writer thread, after metadata identity.
    /// Cancellation after fence acquisition cannot release it before the callback finishes.
    pub(crate) async fn history_transaction<T, F>(
        &self,
        generation: Generation,
        callback: F,
    ) -> Result<T, CacheError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T, CacheError> + Send + 'static,
    {
        self.history_transaction_current(generation, CancellationToken::new(), callback)
            .await
    }

    /// Bind queued model work to its request epoch, including cancellation that occurs
    /// after admission but before the shared writer executes the callback.
    pub(super) async fn history_transaction_current<T, F>(
        &self,
        generation: Generation,
        cancel: CancellationToken,
        callback: F,
    ) -> Result<T, CacheError>
    where
        T: Send + 'static,
        F: FnOnce(&Transaction<'_>) -> Result<T, CacheError> + Send + 'static,
    {
        if cancel.is_cancelled() {
            return Err(CacheError::Superseded);
        }
        self.claim(generation)?;
        let fence = self.fence().await?;
        let context = self.context();
        tokio::spawn(async move {
            if cancel.is_cancelled() {
                return Err(CacheError::Superseded);
            }
            context.validate(generation)?;
            let writer = context.writer.clone();
            let result = writer
                .cache_write(move |transaction| {
                    super::history_rows::verify(transaction, context.account)?;
                    context.validate(generation)?;
                    if cancel.is_cancelled() {
                        return Err(CacheError::Superseded);
                    }
                    let output = callback(transaction)?;
                    context.validate(generation)?;
                    if cancel.is_cancelled() {
                        return Err(CacheError::Superseded);
                    }
                    Ok(output)
                })
                .await;
            drop(fence);
            result
        })
        .await
        .map_err(|_| CacheError::TaskStopped)?
    }

    /// Authorize a backend-verified visible channel in this generation. Callers must have
    /// verified identity and channel authority; cached resource visibility alone is not a grant.
    /// `fresh` retires even a live set, for bounded replacement of oversized model deltas.
    pub(crate) async fn history_authorize(
        &self,
        channel: [u8; 16],
        generation: Generation,
        fresh: bool,
    ) -> Result<i64, CacheError> {
        self.history_transaction(generation, move |transaction| {
            if !visible(transaction, channel)? {
                return Err(CacheError::InvalidData);
            }
            let generation = generation_sql(generation)?;
            if !fresh && let Some(set_id) = current(transaction, channel)? {
                transaction.execute("UPDATE history_channels SET verified_generation=?1 WHERE channel_id=?2", params![generation, channel.as_slice()]).map_err(WorkerError::from)?;
                return Ok(set_id);
            }
            revoke(transaction, channel)?;
            transaction.execute("INSERT INTO history_sets (channel_id) VALUES (?1)", [channel.as_slice()]).map_err(WorkerError::from)?;
            let set_id = transaction.last_insert_rowid();
            transaction.execute("INSERT INTO history_channels (channel_id,current_set,verified_generation) VALUES (?1,?2,?3) ON CONFLICT(channel_id) DO UPDATE SET current_set=excluded.current_set,verified_generation=excluded.verified_generation", params![channel.as_slice(), set_id, generation]).map_err(WorkerError::from)?;
            transaction.execute("INSERT INTO history_manifest (revision,channel_id,set_id) SELECT history_revision,?1,?2 FROM cache_metadata WHERE singleton=1", params![channel.as_slice(), set_id]).map_err(WorkerError::from)?;
            Ok(set_id)
        }).await
    }

    /// Atomically persist an opaque incremental delta and its small model metadata.
    /// At most 499 upserts/deletes are allowed, leaving one changed row for metadata.
    /// Revoked, unverified or superseded incarnations are never writable.
    #[cfg(test)]
    pub(crate) async fn history_commit(
        &self,
        channel: [u8; 16],
        set_id: i64,
        generation: Generation,
        metadata: Vec<u8>,
        upserts: Vec<HistoryRow>,
        deletes: Vec<HistoryKey>,
    ) -> Result<(), CacheError> {
        self.history_commit_current(
            channel,
            set_id,
            (generation, CancellationToken::new()),
            metadata,
            upserts,
            deletes,
        )
        .await
    }

    /// Commit a model delta only while its dispatched request epoch is still current.
    /// The token is checked on the actual writer thread before and after mutation.
    pub(crate) async fn history_commit_current(
        &self,
        channel: [u8; 16],
        set_id: i64,
        current: (Generation, CancellationToken),
        metadata: Vec<u8>,
        upserts: Vec<HistoryRow>,
        deletes: Vec<HistoryKey>,
    ) -> Result<(), CacheError> {
        let (generation, cancel) = current;
        if upserts.len().saturating_add(deletes.len()) > 499 {
            return Err(WorkerError::RowLimit.into());
        }
        self.history_transaction_current(generation, cancel, move |transaction| {
            if authorized(transaction, channel, generation)? != Some(set_id) {
                return Err(CacheError::InvalidData);
            }
            for key in deletes {
                key.table.delete(transaction, set_id, key.id)?;
            }
            for row in upserts {
                row.table.upsert(transaction, set_id, &row)?;
            }
            transaction
                .execute(
                    "UPDATE history_sets SET metadata=?1 WHERE id=?2",
                    params![metadata, set_id],
                )
                .map_err(WorkerError::from)?;
            Ok(())
        })
        .await
    }

    /// Hide history immediately in one cache transaction. Physical cleanup is separate;
    /// successful reauthorization always allocates a new incarnation after this marker.
    pub(crate) async fn history_revoke(
        &self,
        channel: [u8; 16],
        generation: Generation,
    ) -> Result<(), CacheError> {
        self.history_transaction(generation, move |transaction| revoke(transaction, channel))
            .await
    }

    /// Validate a model's in-memory page against durable markers without loading payloads.
    /// A missing, revoked or different incarnation is rejected before cached data is returned.
    pub(crate) async fn history_validate(
        &self,
        channel: [u8; 16],
        set_id: i64,
        generation: Generation,
    ) -> Result<(), CacheError> {
        self.claim(generation)?;
        let fence = self.fence().await?;
        let context = self.context();
        tokio::spawn(async move {
            context.validate(generation)?;
            let check = context.generation.clone();
            let (fence, ()) = context
                .query(fence, move |transaction| {
                    validate_generation(&check, generation)?;
                    if authorized(transaction, channel, generation)? != Some(set_id) {
                        return Err(CacheError::InvalidData);
                    }
                    Ok(())
                })
                .await?;
            context.validate(generation)?;
            drop(fence);
            Ok(())
        })
        .await
        .map_err(|_| CacheError::TaskStopped)?
    }

    /// Read a coherent incarnation through the bounded read pool. Every <=500-row batch
    /// rechecks identity, generation, current resource visibility and durable revocation.
    pub(crate) async fn history_load(
        &self,
        channel: [u8; 16],
        generation: Generation,
    ) -> Result<Option<HistoryData>, CacheError> {
        self.claim(generation)?;
        let fence = self.fence().await?;
        let context = self.context();
        tokio::spawn(async move {
            context.validate(generation)?;
            let check = context.generation.clone();
            let (mut fence, initial) = context
                .query(fence, move |transaction| {
                    validate_generation(&check, generation)?;
                    let Some(set_id) = authorized(transaction, channel, generation)? else {
                        return Ok(None);
                    };
                    let metadata = transaction
                        .query_row(
                            "SELECT metadata FROM history_sets WHERE id=?1",
                            [set_id],
                            |row| row.get(0),
                        )
                        .map_err(WorkerError::from)?;
                    Ok(Some(HistoryData {
                        set_id,
                        metadata,
                        rows: Vec::new(),
                    }))
                })
                .await?;
            let Some(mut data) = initial else {
                return Ok(None);
            };
            for table in TABLES {
                let mut after = None;
                loop {
                    context.validate(generation)?;
                    let set_id = data.set_id;
                    let check = context.generation.clone();
                    let (returned, rows) = context
                        .query(fence, move |transaction| {
                            validate_generation(&check, generation)?;
                            table.read(transaction, channel, generation, set_id, after)
                        })
                        .await?;
                    fence = returned;
                    let count = rows.len();
                    after = rows.last().map(|row| row.id);
                    data.rows.extend(rows);
                    if count < 500 {
                        break;
                    }
                }
            }
            context.validate(generation)?;
            drop(fence);
            Ok(Some(data))
        })
        .await
        .map_err(|_| CacheError::TaskStopped)?
    }
}

pub(super) fn validate_generation(
    current: &std::sync::atomic::AtomicU64,
    generation: Generation,
) -> Result<(), CacheError> {
    if current.load(std::sync::atomic::Ordering::Acquire) != generation.get() {
        return Err(CacheError::Superseded);
    }
    Ok(())
}

fn generation_sql(generation: Generation) -> Result<i64, CacheError> {
    i64::try_from(generation.get()).map_err(|_| CacheError::InvalidData)
}

pub(super) fn visible(connection: &Connection, channel: [u8; 16]) -> Result<bool, CacheError> {
    connection.query_row("SELECT EXISTS(SELECT 1 FROM channels WHERE id=?1 AND revision=(SELECT channels_revision FROM cache_metadata WHERE singleton=1) UNION ALL SELECT 1 FROM dms WHERE id=?1 AND revision=(SELECT dms_revision FROM cache_metadata WHERE singleton=1))", [channel.as_slice()], |row| row.get(0)).map_err(WorkerError::from).map_err(CacheError::from)
}

fn current(connection: &Connection, channel: [u8; 16]) -> Result<Option<i64>, CacheError> {
    if !visible(connection, channel)? {
        return Ok(None);
    }
    connection.query_row("SELECT s.id FROM history_channels h JOIN history_sets s ON s.id=h.current_set AND s.channel_id=h.channel_id JOIN history_manifest m ON m.set_id=s.id AND m.channel_id=h.channel_id WHERE h.channel_id=?1 AND s.revoked=0 AND m.revision=(SELECT history_revision FROM cache_metadata WHERE singleton=1)", [channel.as_slice()], |row| row.get(0)).optional().map_err(WorkerError::from).map_err(CacheError::from)
}

pub(super) fn authorized(
    connection: &Connection,
    channel: [u8; 16],
    generation: Generation,
) -> Result<Option<i64>, CacheError> {
    let verified: Option<i64> = connection
        .query_row(
            "SELECT verified_generation FROM history_channels WHERE channel_id=?1",
            [channel.as_slice()],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()
        .map_err(WorkerError::from)?
        .flatten();
    if verified != Some(generation_sql(generation)?) {
        return Ok(None);
    }
    current(connection, channel)
}

fn revoke(transaction: &Transaction<'_>, channel: [u8; 16]) -> Result<(), CacheError> {
    transaction.execute("UPDATE history_sets SET revoked=1 WHERE id=(SELECT current_set FROM history_channels WHERE channel_id=?1) AND revoked=0", [channel.as_slice()]).map_err(WorkerError::from)?;
    transaction.execute("UPDATE history_channels SET current_set=NULL WHERE channel_id=?1 AND current_set IS NOT NULL", [channel.as_slice()]).map_err(WorkerError::from)?;
    transaction.execute("DELETE FROM history_manifest WHERE channel_id=?1 AND revision=(SELECT history_revision FROM cache_metadata WHERE singleton=1)", [channel.as_slice()]).map_err(WorkerError::from)?;
    Ok(())
}
