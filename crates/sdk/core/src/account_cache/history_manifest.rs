//! Bounded history visibility staging; resource publication atomically switches the durable marker.

use super::{CacheContext, CacheError, history, history_rows};
use crate::{Generation, storage_worker::WorkerError};
use rusqlite::params;
use std::sync::Arc;

/// Stage the next visibility manifest without changing the currently visible incarnation.
/// Existing active membership, not merely a lingering current-set pointer, is required:
/// disappearance followed by regrant must never revive an old partially purged set.
pub(super) async fn stage(
    context: &CacheContext,
    revision: i64,
    channels_revision: i64,
    dms_revision: i64,
    generation: Generation,
) -> Result<(), CacheError> {
    let mut after: Option<Vec<u8>> = None;
    loop {
        context.validate(generation)?;
        let check = Arc::clone(&context.generation);
        let account = context.account;
        let batch = context.writer.cache_write(move |transaction| {
            history_rows::verify(transaction, account)?;
            history::validate_generation(&check, generation)?;
            let mut statement = transaction.prepare(
                "SELECT h.channel_id,h.current_set FROM history_channels h
                JOIN history_sets s ON s.id=h.current_set AND s.revoked=0
                JOIN history_manifest m ON m.channel_id=h.channel_id AND m.set_id=s.id
                WHERE m.revision=(SELECT history_revision FROM cache_metadata WHERE singleton=1)
                  AND (?1 IS NULL OR h.channel_id>?1)
                  AND (EXISTS(SELECT 1 FROM channels c WHERE c.id=h.channel_id AND c.revision=?2)
                    OR EXISTS(SELECT 1 FROM dms d WHERE d.id=h.channel_id AND d.revision=?3))
                ORDER BY h.channel_id LIMIT 500"
            ).map_err(WorkerError::from)?;
            let rows = statement.query_map(params![after, channels_revision, dms_revision], |row| {
                Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, i64>(1)?))
            }).map_err(WorkerError::from)?.collect::<rusqlite::Result<Vec<_>>>().map_err(WorkerError::from)?;
            for (channel, set_id) in &rows {
                transaction.execute("INSERT INTO history_manifest (revision,channel_id,set_id) VALUES (?1,?2,?3)", params![revision, channel, set_id]).map_err(WorkerError::from)?;
            }
            history::validate_generation(&check, generation)?;
            Ok::<_, CacheError>(rows)
        }).await?;
        let count = batch.len();
        after = batch.last().map(|(channel, _)| channel.clone());
        if count < 500 {
            return Ok(());
        }
    }
}
