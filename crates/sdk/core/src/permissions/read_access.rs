//! Effective cached-history read decision per channel for one account, derived from one accepted
//! resource snapshot through the evaluator above. History applies the transitions; the backend
//! still authorizes every request.

use std::collections::{BTreeMap, BTreeSet};

use papo_api::Uuid;

use super::{AccountEvidence, PermissionDataError, direct_channel_grants};
use crate::ResourceSnapshot;

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
    pub(crate) fn evaluate(snapshot: &ResourceSnapshot, user: Uuid) -> Self {
        let mut result = Self::default();
        let evidence = AccountEvidence::new(snapshot, user);
        for channel in snapshot.channels.iter() {
            let id = channel.id;
            let access = match evidence.channel_grants(channel) {
                Ok(grants) if grants.read => ReadAccess::Allowed,
                Ok(_) => ReadAccess::Denied,
                Err(PermissionDataError::Unavailable) => ReadAccess::Unknown,
            };
            result.insert(id, access);
        }
        for dm in snapshot.dms.iter() {
            let access = match direct_channel_grants(snapshot, dm.id) {
                Ok(grants) if grants.read => ReadAccess::Allowed,
                Ok(_) => ReadAccess::Denied,
                Err(PermissionDataError::Unavailable) => ReadAccess::Unknown,
            };
            result.insert(dm.id, access);
        }
        result
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

#[cfg(test)]
mod tests;
