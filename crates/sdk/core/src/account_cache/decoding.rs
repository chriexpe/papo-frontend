//! Decodes one visible v2 revision for display. Account and SQLite integrity failures
//! remain cache errors. A schema-valid legacy group that fails admission rebuilds on reconnect.

use super::{
    CacheContext, CacheError, Fence,
    schema::{self, TABLES},
};
use crate::{
    Generation, ResourceSnapshot, Uuid,
    resource_admission::{Admission, Diagnostics},
    resources::{AdmittedMutation, ProfileBatch},
    storage_worker::WorkerError,
};
use papo_api::wire::{
    self, BlockListResponse, ChannelList, DirectConversationList, RoleListResponse,
};
use rusqlite::params;
use serde::{Deserialize, de::DeserializeOwned};

/// Keys independently stored in SQLite, in backend order.
struct Rows<T> {
    keys: Vec<Uuid>,
    values: Vec<T>,
}

/// A relationship may outlive the user summary it once contained. Old rows have a
/// materialized nested summary and no marker.
#[derive(Deserialize)]
struct Relationship<T> {
    #[serde(flatten)]
    value: T,
    #[serde(default)]
    summary_unavailable: bool,
}

/// Profiles written from the normalized owner also record the known ban flag for a
/// person whose only remaining appearance is a requested profile.
#[derive(Deserialize)]
struct Profile {
    #[serde(flatten)]
    value: wire::UserProfile,
    banned: Option<bool>,
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
    let reader = RevisionReader {
        context,
        generation,
        revisions: metadata.revisions,
    };
    let (fence, server) = reader
        .group(fence, 0, |row: &wire::Server| row.id.as_deref())
        .await?;
    let (fence, channels) = reader
        .group(fence, 1, |row: &wire::Channel| row.id.as_deref())
        .await?;
    let (fence, roles) = reader
        .group(fence, 2, |row: &wire::Role| row.id.as_deref())
        .await?;
    let (fence, members) = reader
        .group(fence, 3, |row: &wire::UserSummary| row.id.as_deref())
        .await?;
    let (fence, profiles) = reader
        .group(fence, 4, |row: &Profile| row.value.id.as_deref())
        .await?;
    let (fence, dms) = reader
        .group(fence, 5, |row: &Relationship<wire::DirectConversation>| {
            Some(row.value.id.as_str())
        })
        .await?;
    let (fence, blocks) = reader
        .group(fence, 6, |row: &Relationship<wire::UserSummary>| {
            row.value.id.as_deref()
        })
        .await?;
    let snapshot = tokio::task::spawn_blocking(move || {
        let diagnostics = Diagnostics::default();
        let own = account.user();
        let admit = Admission::for_disk(own, &diagnostics);
        let mut base = ResourceSnapshot::empty(generation);
        base.has_display_baseline = true;
        if let Some(rows) = roles
            && let Ok(rows) = admit.roles(RoleListResponse {
                roles: Some(rows.values),
            })
        {
            base.roles.replace(rows, |role| role.id);
        }
        // Legacy rows can contain a different copy of one summary per relationship.
        // Applying the admitted groups together gives member evidence precedence and
        // keeps profile-exclusive fields separate from that resolution.
        let mut mutation = AdmittedMutation::default();
        if let Some(mut rows) = server
            && rows.values.len() == 1
        {
            mutation.server = rows.values.pop().and_then(|row| admit.server(row).ok());
        }
        mutation.channels = channels.and_then(|rows| {
            admit
                .channels(ChannelList {
                    channels: Some(rows.values),
                })
                .ok()
        });
        mutation.roster = members.and_then(|rows| admit.cached_members(rows.values).ok());
        if let Some(rows) = profiles
            && let Some(batch) = admit_profiles(rows, admit)
        {
            mutation.profiles.push(batch);
        }
        let unavailable = admit_relationships(&mut mutation, admit, dms, blocks);
        let mut snapshot = mutation
            .apply(&base, own)
            .map_err(|_| CacheError::InvalidData)?;
        for id in unavailable {
            if !snapshot.members.contains(id) && snapshot.profiles.get(id).is_none() {
                snapshot.users.remove(id);
            }
        }
        snapshot.version = metadata.version;
        Ok::<_, CacheError>(snapshot)
    })
    .await
    .map_err(|_| CacheError::TaskStopped)??;
    context.validate(generation)?;
    drop(fence);
    Ok(snapshot)
}

/// SQLite keys are unique across the group. Admitting each keyed profile directly
/// avoids checking every row against an unbounded requested-ID list on restart.
fn admit_profiles(rows: Rows<Profile>, admit: Admission<'_>) -> Option<ProfileBatch> {
    let mut present = Vec::with_capacity(rows.keys.len());
    for (key, row) in rows.keys.into_iter().zip(rows.values) {
        let (detail, mut user) = admit.profile(key, row.value).ok()?;
        user.banned = row.banned;
        present.push((detail, user));
    }
    Some(ProfileBatch {
        present,
        absent: Vec::new(),
    })
}

fn admit_relationships(
    mutation: &mut AdmittedMutation,
    admit: Admission<'_>,
    dms: Option<Rows<Relationship<wire::DirectConversation>>>,
    blocks: Option<Rows<Relationship<wire::UserSummary>>>,
) -> Vec<Uuid> {
    let mut unavailable = Vec::new();
    if let Some(rows) = dms {
        let (missing, values) = split_relationships(rows);
        if let Ok(rows) = admit.dms(DirectConversationList { dms: values }) {
            unavailable.extend(
                rows.iter()
                    .zip(missing)
                    .filter_map(|((dm, _), absent)| absent.then_some(dm.peer_id)),
            );
            mutation.dms = Some(rows);
        }
    }
    if let Some(rows) = blocks {
        let (missing, values) = split_relationships(rows);
        if let Ok(rows) = admit.blocks(BlockListResponse { users: values }) {
            unavailable.extend(
                rows.iter()
                    .zip(missing)
                    .filter_map(|(user, absent)| absent.then_some(user.id)),
            );
            mutation.blocks = Some(rows);
        }
    }
    unavailable
}

fn split_relationships<T>(rows: Rows<Relationship<T>>) -> (Vec<bool>, Vec<T>) {
    rows.values
        .into_iter()
        .map(|row| (row.summary_unavailable, row.value))
        .unzip()
}

/// All seven display groups read pointers from the same checked metadata revision.
struct RevisionReader<'a> {
    context: &'a CacheContext,
    generation: Generation,
    revisions: [i64; schema::TABLE_COUNT],
}

impl RevisionReader<'_> {
    async fn group<T: DeserializeOwned + Send + 'static>(
        &self,
        mut fence: Fence,
        table: usize,
        payload_id: fn(&T) -> Option<&str>,
    ) -> Result<(Fence, Option<Rows<T>>), CacheError> {
        let revision = self.revisions[table];
        if revision == 0 {
            return Ok((fence, None));
        }
        let mut values = Vec::new();
        let mut keys = Vec::new();
        let mut next = 0_i64;
        loop {
            self.context.validate(self.generation)?;
            let (returned_fence, batch) = self
                .context
                .query(fence, move |transaction| {
                    let mut statement = transaction
                        .prepare(TABLES[table].select)
                        .map_err(WorkerError::from)?;
                    let mut rows = statement
                        .query(params![revision, next])
                        .map_err(WorkerError::from)?;
                    let mut batch = Vec::new();
                    while let Some(row) = rows.next().map_err(WorkerError::from)? {
                        let position = row.get::<_, i64>(0).map_err(WorkerError::from)?;
                        let id = Uuid::from_bytes(row.get(1).map_err(WorkerError::from)?);
                        let bytes = row.get::<_, Vec<u8>>(2).map_err(WorkerError::from)?;
                        let value = match serde_json::from_slice::<T>(&bytes) {
                            Ok(value) => value,
                            Err(error) if error.is_data() => return Ok(None),
                            Err(_) => return Err(CacheError::InvalidData),
                        };
                        if payload_id(&value)
                            != Some(id.hyphenated().encode_lower(&mut Uuid::encode_buffer()))
                        {
                            return Ok(None);
                        }
                        batch.push((position, id, value));
                    }
                    Ok(Some(batch))
                })
                .await?;
            fence = returned_fence;
            let Some(batch) = batch else {
                return Ok((fence, None));
            };
            let count = batch.len();
            for (position, id, value) in batch {
                next = position.checked_add(1).ok_or(CacheError::InvalidData)?;
                keys.push(id);
                values.push(value);
            }
            if count < 500 {
                return Ok((fence, Some(Rows { keys, values })));
            }
        }
    }
}
