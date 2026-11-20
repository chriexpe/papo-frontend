//! Resource payloads and role edges for a SQLite revision. SQL visibility lives in persistence.

use super::{CacheError, CacheResource, ResourceSnapshot, schema::MEMBER_ROLES};
use crate::Uuid;
use serde::Serialize;

mod payload;

pub(super) struct Selection {
    snapshot: ResourceSnapshot,
    resources: Vec<CacheResource>,
}

pub(super) struct Encoded {
    pub version: i64,
    pub tables: Vec<(usize, Vec<Record>)>,
}

pub(super) struct Record {
    pub id: [u8; 16],
    pub parent: Option<[u8; 16]>,
    pub payload: Vec<u8>,
}

/// The immutable roots share their entity storage with the caller. Encoding joins only
/// selected relations to canonical users and never copies unrelated records.
pub(super) fn select(snapshot: &ResourceSnapshot, resources: &[CacheResource]) -> Selection {
    let mut selected = Vec::with_capacity(resources.len());
    for &resource in resources {
        if !selected.contains(&resource) {
            selected.push(resource);
        }
    }
    Selection {
        snapshot: snapshot.clone(),
        resources: selected,
    }
}

pub(super) fn encode(selection: Selection) -> Result<Encoded, CacheError> {
    let Selection {
        snapshot,
        resources,
    } = selection;
    let version = i64::try_from(snapshot.version).map_err(|_| CacheError::InvalidData)?;
    let mut tables = Vec::with_capacity(resources.len() + 1);
    for resource in resources {
        let (index, records) = match resource {
            CacheResource::Server => (
                0,
                snapshot
                    .server
                    .iter()
                    .map(|server| record(server.id, server.as_ref()))
                    .collect::<Result<_, _>>()?,
            ),
            CacheResource::Channels => (
                1,
                snapshot
                    .channels
                    .iter()
                    .map(|channel| record(channel.id, &payload::channel(channel)))
                    .collect::<Result<_, _>>()?,
            ),
            CacheResource::Roles => (
                2,
                snapshot
                    .roles
                    .iter()
                    .map(|role| record(role.id, role))
                    .collect::<Result<_, _>>()?,
            ),
            CacheResource::Members => {
                let mut members = Vec::with_capacity(snapshot.members.len());
                let mut edges = Vec::new();
                for id in snapshot.members.iter() {
                    let user = snapshot.users.get(id).ok_or(CacheError::InvalidData)?;
                    members.push(record(id, &payload::summary(id, Some(user)))?);
                    for role in user.roles.iter().flatten() {
                        edges.push(Record {
                            id: *role.as_bytes(),
                            parent: Some(*id.as_bytes()),
                            payload: Vec::new(),
                        });
                    }
                }
                tables.push((MEMBER_ROLES, edges));
                (3, members)
            }
            CacheResource::Profiles => (
                4,
                snapshot
                    .profiles
                    .iter()
                    .map(|detail| {
                        let user = snapshot.users.get(detail.id);
                        record(detail.id, &payload::profile(detail, user))
                    })
                    .collect::<Result<_, _>>()?,
            ),
            CacheResource::Dms => (
                5,
                snapshot
                    .dms
                    .iter()
                    .map(|dm| {
                        let user = snapshot.users.get(dm.peer_id);
                        record(dm.id, &payload::direct(dm, user))
                    })
                    .collect::<Result<_, _>>()?,
            ),
            CacheResource::Blocks => (
                6,
                snapshot
                    .blocks
                    .iter()
                    .map(|id| {
                        let user = snapshot.users.get(id);
                        record(id, &payload::blocked(id, user))
                    })
                    .collect::<Result<_, _>>()?,
            ),
        };
        tables.push((index, records));
    }
    Ok(Encoded { version, tables })
}

fn record(id: Uuid, item: &impl Serialize) -> Result<Record, CacheError> {
    Ok(Record {
        id: *id.as_bytes(),
        parent: None,
        payload: serde_json::to_vec(item).map_err(|_| CacheError::InvalidData)?,
    })
}
