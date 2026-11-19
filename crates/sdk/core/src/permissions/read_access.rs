//! Effective cached-history read decision per channel for one account, derived from one accepted
//! resource snapshot through the evaluator above. History applies the transitions; the backend
//! still authorizes every request.

use std::collections::{BTreeMap, BTreeSet};

use papo_api::Uuid;

use super::{AccountEvidence, PermissionDataError, direct_grants};
use crate::{ClientError, ResourceSnapshot};

/// What accepted resource evidence says about reading one channel's history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadAccess {
    /// Sufficient admitted evidence grants `read_channel` (or DM membership without a block).
    Allowed,
    /// Admitted evidence is complete and denies reading.
    Denied,
    /// A required input is absent or unattributable; never treated as a grant or a removal.
    Unknown,
}

/// Decisions for every channel and DM listed in one snapshot. A channel that is not listed
/// has no entry; callers treat that as removal, exactly as an accepted list replacement.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct ReadDecisions {
    decisions: BTreeMap<Uuid, ReadAccess>,
}

impl ReadDecisions {
    /// Evaluate every listed channel for `user`.
    /// # Errors
    /// Returns `ResourceData` when a listed channel or DM has no parseable ID: dropping it
    /// would turn a malformed row into an apparent removal.
    pub(crate) fn evaluate(snapshot: &ResourceSnapshot, user: Uuid) -> Result<Self, ClientError> {
        let mut result = Self::default();
        let evidence = AccountEvidence::new(snapshot, user);
        for channel in snapshot.channels.iter() {
            let id = parse(channel.id.as_deref())?;
            let access = match evidence.channel_grants(channel) {
                Ok(grants) if grants.read => ReadAccess::Allowed,
                Ok(_) => ReadAccess::Denied,
                Err(PermissionDataError::Unavailable) => ReadAccess::Unknown,
            };
            result.insert(id, access);
        }
        let blocked = own_blocks(snapshot);
        for dm in snapshot.dms.iter() {
            let id = parse(Some(dm.id.as_str()))?;
            result.insert(id, direct_access(dm, blocked.as_ref()));
        }
        Ok(result)
    }

    pub(crate) fn get(&self, channel: Uuid) -> Option<ReadAccess> {
        self.decisions.get(&channel).copied()
    }

    pub(crate) fn with(&self, access: ReadAccess) -> BTreeSet<Uuid> {
        self.decisions
            .iter()
            .filter(|(_, decision)| **decision == access)
            .map(|(id, _)| *id)
            .collect()
    }

    /// An ID listed twice has no single decision, so neither copy may grant.
    fn insert(&mut self, id: Uuid, access: ReadAccess) {
        if self.decisions.insert(id, access).is_some() {
            self.decisions.insert(id, ReadAccess::Unknown);
        }
    }
}

fn parse(id: Option<&str>) -> Result<Uuid, ClientError> {
    id.and_then(|id| id.parse().ok())
        .ok_or(ClientError::ResourceData)
}

/// The account's own blocks, or `None` when an entry cannot be attributed to a user.
fn own_blocks(snapshot: &ResourceSnapshot) -> Option<BTreeSet<Uuid>> {
    snapshot
        .blocks
        .iter()
        .map(|user| user.id.as_deref().and_then(|id| id.parse().ok()))
        .collect()
}

/// The pinned DM list only returns conversations where the account is a member, the
/// account has not hidden it and there is no block in either direction. A listed DM is
/// membership evidence. The own block list can add a block (lists fetched separately
/// may disagree) but cannot prove that the peer has not blocked the account.
fn direct_access(
    dm: &papo_api::wire::DirectConversation,
    blocked: Option<&BTreeSet<Uuid>>,
) -> ReadAccess {
    let peer = dm.user.id.as_deref().and_then(|id| id.parse::<Uuid>().ok());
    let (Some(peer), Some(blocked)) = (peer, blocked) else {
        return ReadAccess::Unknown;
    };
    if direct_grants(true, blocked.contains(&peer)).read {
        ReadAccess::Allowed
    } else {
        ReadAccess::Denied
    }
}

#[cfg(test)]
mod tests;
