//! Bounded revision staging, atomic publication and batched reads; account ownership lives in the factory.

use super::{
    CacheContext, CacheError, Fence,
    encoding::Encoded,
    schema::{self, TABLES},
};
use crate::{
    Generation, ResourceSnapshot,
    storage_worker::{SqlBatch, SqlMutation, WorkerError},
};
use papo_api::wire::{Channel, DirectConversation, Role, Server, UserProfile, UserSummary};
use rusqlite::{params, types::Value};
use serde::de::DeserializeOwned;
use std::sync::Arc;

pub(super) async fn replace(
    context: &CacheContext,
    fence: Fence,
    encoded: Encoded,
    generation: Generation,
) -> Result<(), CacheError> {
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
    for table in &TABLES {
        loop {
            let result = context
                .writer
                .commit(SqlBatch::new(vec![SqlMutation::new(
                    table.prune,
                    Vec::new(),
                )])?)
                .await?;
            if result.affected_rows < 500 {
                break;
            }
        }
    }
    super::history_purge::purge(context).await?;
    context.writer.checkpoint().await?;
    drop(fence);
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

pub(super) async fn load(
    context: &CacheContext,
    fence: Fence,
    generation: Generation,
) -> Result<ResourceSnapshot, CacheError> {
    context.validate(generation)?;
    let account = context.account;
    let (fence, metadata) = context
        .query(fence, move |transaction| {
            schema::metadata(transaction, account)
                .map_err(WorkerError::from)
                .map_err(CacheError::from)
        })
        .await?;
    let mut snapshot = ResourceSnapshot::empty(generation);
    snapshot.version = metadata.version;
    let (fence, server) =
        read_values::<Server>(context, fence, 0, metadata.revisions[0], generation).await?;
    snapshot.server = server.into_iter().next().map(Arc::new);
    let (fence, channels) =
        read_values::<Channel>(context, fence, 1, metadata.revisions[1], generation).await?;
    snapshot.channels = channels.into();
    let (fence, roles) =
        read_values::<Role>(context, fence, 2, metadata.revisions[2], generation).await?;
    snapshot.roles = roles.into();
    let (fence, members) =
        read_values::<UserSummary>(context, fence, 3, metadata.revisions[3], generation).await?;
    snapshot.members = members.into();
    let (fence, profiles) =
        read_values::<UserProfile>(context, fence, 4, metadata.revisions[4], generation).await?;
    snapshot.profiles = profiles.into();
    let (fence, dms) =
        read_values::<DirectConversation>(context, fence, 5, metadata.revisions[5], generation)
            .await?;
    snapshot.dms = dms.into();
    let (_fence, blocks) =
        read_values::<UserSummary>(context, fence, 6, metadata.revisions[6], generation).await?;
    snapshot.blocks = blocks.into();
    context.validate(generation)?;
    Ok(snapshot)
}

async fn read_values<T: DeserializeOwned + Send + 'static>(
    context: &CacheContext,
    mut fence: Fence,
    table: usize,
    revision: i64,
    generation: Generation,
) -> Result<(Fence, Vec<T>), CacheError> {
    let mut values = Vec::new();
    let mut position = 0_i64;
    loop {
        context.validate(generation)?;
        let (returned_fence, batch) = context
            .query(fence, move |transaction| {
                let mut statement = transaction
                    .prepare(TABLES[table].select)
                    .map_err(WorkerError::from)?;
                let payloads = statement
                    .query_map(params![revision, position], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
                    })
                    .map_err(WorkerError::from)?;
                payloads
                    .map(|payload| {
                        let (position, payload) = payload.map_err(WorkerError::from)?;
                        let value = serde_json::from_slice::<T>(&payload)
                            .map_err(|_| CacheError::InvalidData)?;
                        Ok((position, value))
                    })
                    .collect::<Result<Vec<_>, CacheError>>()
            })
            .await?;
        fence = returned_fence;
        let count = batch.len();
        if let Some((last_position, _)) = batch.last() {
            position = last_position
                .checked_add(1)
                .ok_or(CacheError::InvalidData)?;
        }
        values.extend(batch.into_iter().map(|(_, value)| value));
        if count < 500 {
            return Ok((fence, values));
        }
    }
}
