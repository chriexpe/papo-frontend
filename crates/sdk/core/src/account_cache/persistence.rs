//! Bounded revision staging, atomic publication and batched reads; account ownership lives in the factory.

use super::{
    CacheContext, CacheError, Fence,
    encoding::Encoded,
    schema::{self, TABLES},
};
use crate::{
    Generation,
    storage_worker::{SqlBatch, SqlMutation, WorkerError},
};
use rusqlite::types::Value;
use std::sync::Arc;

pub(super) async fn replace(
    context: &CacheContext,
    fence: Fence,
    encoded: Encoded,
    generation: Generation,
) -> Result<Option<CacheError>, CacheError> {
    context.validate(generation)?;
    let account = context.account;
    let (fence, metadata) = context
        .query(fence, move |transaction| {
            schema::metadata(transaction, account)
                .map_err(WorkerError::from)
                .map_err(CacheError::from)
        })
        .await?;
    let revision = metadata.next_revision;
    let next_revision = revision.checked_add(1).ok_or(CacheError::InvalidData)?;
    context
        .writer
        .commit(SqlBatch::new(vec![SqlMutation::new(
            "UPDATE cache_metadata SET next_revision=?1 WHERE singleton=1",
            vec![Value::Integer(next_revision)],
        )])?)
        .await?;
    for (table_index, records) in &encoded.tables {
        let table = &TABLES[*table_index];
        for (batch_index, chunk) in records.chunks(500).enumerate() {
            let mut mutations = Vec::with_capacity(chunk.len());
            for (index, record) in chunk.iter().enumerate() {
                let position = batch_index
                    .checked_mul(500)
                    .and_then(|start| start.checked_add(index))
                    .and_then(|position| i64::try_from(position).ok())
                    .ok_or(CacheError::InvalidData)?;
                let parameters = if let Some(parent) = record.parent {
                    vec![
                        Value::Integer(revision),
                        Value::Blob(record.id.to_vec()),
                        Value::Blob(parent.to_vec()),
                        Value::Integer(position),
                    ]
                } else {
                    vec![
                        Value::Integer(revision),
                        Value::Blob(record.id.to_vec()),
                        Value::Integer(position),
                        Value::Blob(record.payload.clone()),
                    ]
                };
                mutations.push(SqlMutation::new(table.insert, parameters));
            }
            context.writer.commit(SqlBatch::new(mutations)?).await?;
            context.validate(generation)?;
        }
    }
    publish(context, &encoded, &metadata, generation).await?;
    // After the pointer switch, maintenance failure cannot roll the resource revision back.
    let maintenance_error = clean_obsolete_rows(context).await.err();
    drop(fence);
    Ok(maintenance_error)
}

async fn clean_obsolete_rows(context: &CacheContext) -> Result<(), CacheError> {
    for table in &TABLES {
        loop {
            let result = context
                .writer
                .commit(SqlBatch::new(vec![SqlMutation::new(
                    table.prune,
                    Vec::new(),
                )])?)
                .await?;
            if result.affected_rows == 0 {
                break;
            }
        }
    }
    super::history_purge::purge(context).await?;
    context.writer.checkpoint().await?;
    Ok(())
}

/// Publish resource pointers and the durable history marker in one bounded transaction.
async fn publish(
    context: &CacheContext,
    encoded: &Encoded,
    metadata: &schema::Metadata,
    generation: Generation,
) -> Result<(), CacheError> {
    let account = context.account;
    let revision = metadata.next_revision;
    let version = encoded.version;
    let channels_changed = encoded.tables.iter().any(|(index, _)| *index == 1);
    let dms_changed = encoded.tables.iter().any(|(index, _)| *index == 5);
    let visibility_changed = channels_changed || dms_changed;
    if visibility_changed {
        super::history_manifest::stage(
            context,
            revision,
            if channels_changed {
                revision
            } else {
                metadata.revisions[1]
            },
            if dms_changed {
                revision
            } else {
                metadata.revisions[5]
            },
            generation,
        )
        .await?;
    }
    let switch: Vec<_> = encoded
        .tables
        .iter()
        .map(|(index, _)| TABLES[*index].switch)
        .collect();
    let check = Arc::clone(&context.generation);
    context.validate(generation)?;
    context
        .writer
        .cache_write(move |transaction| {
            schema::metadata(transaction, account).map_err(WorkerError::from)?;
            super::history::validate_generation(&check, generation)?;
            for statement in switch {
                transaction
                    .execute(statement, [revision])
                    .map_err(WorkerError::from)?;
            }
            transaction
                .execute(
                    "UPDATE cache_metadata SET snapshot_version=?1 WHERE singleton=1",
                    [version],
                )
                .map_err(WorkerError::from)?;
            if visibility_changed {
                transaction
                    .execute(
                        "UPDATE cache_metadata SET history_revision=?1 WHERE singleton=1",
                        [revision],
                    )
                    .map_err(WorkerError::from)?;
            }
            super::history::validate_generation(&check, generation)
        })
        .await?;
    Ok(())
}
