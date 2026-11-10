//! Cache wire encoding and live-field exclusion; SQL revisions and session generations live elsewhere.

use super::{CacheError, CacheResource, ResourceSnapshot, schema::MEMBER_ROLES};
use papo_api::Uuid;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;

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

/// Copy only selected groups across the owned blocking-work boundary, never presence.
pub(super) fn select(snapshot: &ResourceSnapshot, resources: &[CacheResource]) -> Selection {
    let mut selected = ResourceSnapshot::empty(snapshot.generation);
    selected.version = snapshot.version;
    let mut seen = HashSet::new();
    let resources: Vec<_> = resources
        .iter()
        .copied()
        .filter(|resource| seen.insert(*resource))
        .collect();
    for resource in &resources {
        match resource {
            CacheResource::Server => selected.server.clone_from(&snapshot.server),
            CacheResource::Channels => selected.channels.clone_from(&snapshot.channels),
            CacheResource::Roles => selected.roles.clone_from(&snapshot.roles),
            CacheResource::Members => selected.members.clone_from(&snapshot.members),
            CacheResource::Profiles => selected.profiles.clone_from(&snapshot.profiles),
            CacheResource::Dms => selected.dms.clone_from(&snapshot.dms),
            CacheResource::Blocks => selected.blocks.clone_from(&snapshot.blocks),
        }
    }
    Selection {
        snapshot: selected,
        resources,
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
        let (table, records) = match resource {
            CacheResource::Server => (
                0,
                snapshot
                    .server
                    .iter()
                    .map(|item| record(item.id.as_deref(), item.as_ref()))
                    .collect::<Result<Vec<Record>, _>>()?,
            ),
            CacheResource::Channels => (
                1,
                snapshot
                    .channels
                    .iter()
                    .map(|item| record(item.id.as_deref(), item))
                    .collect::<Result<Vec<Record>, _>>()?,
            ),
            CacheResource::Roles => (
                2,
                snapshot
                    .roles
                    .iter()
                    .map(|item| record(item.id.as_deref(), item))
                    .collect::<Result<Vec<Record>, _>>()?,
            ),
            CacheResource::Members => {
                let records = snapshot
                    .members
                    .iter()
                    .map(|item| record(item.id.as_deref(), item))
                    .collect::<Result<Vec<Record>, _>>()?;
                let mut roles = Vec::new();
                for member in snapshot.members.iter() {
                    let parent = Some(uuid(member.id.as_deref())?);
                    for role in member.roles.iter().flatten() {
                        roles.push(Record {
                            id: uuid(role.id.as_deref())?,
                            parent,
                            payload: Vec::new(),
                        });
                    }
                }
                validate_unique(&roles)?;
                tables.push((MEMBER_ROLES, roles));
                (3, records)
            }
            CacheResource::Profiles => (
                4,
                snapshot
                    .profiles
                    .iter()
                    .map(|item| record(item.id.as_deref(), item))
                    .collect::<Result<Vec<Record>, _>>()?,
            ),
            CacheResource::Dms => (
                5,
                snapshot
                    .dms
                    .iter()
                    .map(|item| record(Some(&item.id), item))
                    .collect::<Result<Vec<Record>, _>>()?,
            ),
            CacheResource::Blocks => (
                6,
                snapshot
                    .blocks
                    .iter()
                    .map(|item| record(item.id.as_deref(), item))
                    .collect::<Result<Vec<Record>, _>>()?,
            ),
        };
        validate_unique(&records)?;
        tables.push((table, records));
    }
    Ok(Encoded { version, tables })
}

fn validate_unique(records: &[Record]) -> Result<(), CacheError> {
    let mut ids = HashSet::with_capacity(records.len());
    if records
        .iter()
        .any(|record| !ids.insert((record.parent, record.id)))
    {
        return Err(CacheError::InvalidData);
    }
    Ok(())
}

fn uuid(value: Option<&str>) -> Result<[u8; 16], CacheError> {
    Uuid::parse_str(value.ok_or(CacheError::InvalidData)?)
        .map(|id| *id.as_bytes())
        .map_err(|_| CacheError::InvalidData)
}

fn record(value: Option<&str>, item: &impl Serialize) -> Result<Record, CacheError> {
    let id = uuid(value)?;
    let mut value = serde_json::to_value(item).map_err(|_| CacheError::InvalidData)?;
    strip_live_fields(&mut value);
    let payload = serde_json::to_vec(&value).map_err(|_| CacheError::InvalidData)?;
    Ok(Record {
        id,
        parent: None,
        payload,
    })
}

fn strip_live_fields(value: &mut Value) {
    match value {
        Value::Object(fields) => {
            for name in [
                "status",
                "status_message",
                "typing",
                "status_updated_at",
                "avatar_blob",
                "icon_blob",
            ] {
                fields.remove(name);
            }
            for value in fields.values_mut() {
                strip_live_fields(value);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_live_fields(item);
            }
        }
        _ => {}
    }
}
