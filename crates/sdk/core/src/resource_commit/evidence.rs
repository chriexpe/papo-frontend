//! Coverage of rejected live evidence. The permission evaluator decides which grants need each input.

use crate::{
    CacheResource, ClientError, ResourceRejection, ResourceSnapshot, Uuid,
    resources::AdmittedMutation,
};
use imbl::OrdSet;
use std::collections::HashSet;

/// Missing proofs on the last-good facts. Only a new observation covering a cause clears it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EvidenceGaps {
    groups: HashSet<CacheResource>,
    /// Named relationships or restrictions observed to change in an incomplete batch.
    channel_membership: OrdSet<Uuid>,
    dm_membership: OrdSet<Uuid>,
    dm_peers: OrdSet<Uuid>,
    own_assignments: OrdSet<Uuid>,
    owner_changed: bool,
    changed_role_flags: OrdSet<Uuid>,
    /// An absent peer needs a fresh social list before a later display summary can be used.
    missing_peers: OrdSet<Uuid>,
    /// Role deletion was observed, but its effect on these channel overwrites is unknown.
    awaiting_channels: OrdSet<Uuid>,
    /// Last-good assignments to these roles predate a failed joint role/channel readback.
    unconfirmed_roles: OrdSet<Uuid>,
}

impl EvidenceGaps {
    pub(crate) fn rejected(&self, group: CacheResource) -> bool {
        self.groups.contains(&group)
    }
    pub(crate) fn unknown(&self, channel: Uuid) -> bool {
        self.channel_membership.contains(&channel)
            || self.dm_membership.contains(&channel)
            || self.dm_peers.contains(&channel)
    }
    pub(crate) fn owner_uncertain(&self) -> bool {
        self.owner_changed || self.rejected(CacheResource::Server)
    }
    pub(crate) fn restriction_uncertain(&self, channel: Uuid) -> bool {
        self.awaiting_channels.contains(&channel)
    }
    pub(crate) fn assignment_uncertain(&self, role: Uuid) -> bool {
        self.own_assignments.contains(&role)
    }
    pub(crate) fn global_roles_uncertain(&self) -> bool {
        !self.own_assignments.is_empty()
    }
    pub(crate) fn role_flags_uncertain(&self, role: Uuid) -> bool {
        self.changed_role_flags.contains(&role) || self.unconfirmed_role(role)
    }
    pub(super) fn census_known(&self) -> bool {
        !self.rejected(CacheResource::Channels)
            && !self.rejected(CacheResource::Dms)
            && self.channel_membership.is_empty()
            && self.dm_membership.is_empty()
    }

    /// Retain provisional facts by the exact input they invalidate. This also covers
    /// channels which are currently denied but could otherwise regrant on a later patch.
    pub(super) fn observe(
        &mut self,
        group: CacheResource,
        base: &ResourceSnapshot,
        mutation: &AdmittedMutation,
        own: Uuid,
    ) {
        match group {
            CacheResource::Server => {
                if let Some(server) = &mutation.server
                    && let Some(old) = &base.server
                {
                    self.owner_changed |=
                        (server.owner_id == Some(own)) != (old.owner_id == Some(own));
                }
            }
            CacheResource::Channels => {
                if let Some(rows) = &mutation.channels {
                    self.channel_membership.extend(membership_changes(
                        base.channels.ids(),
                        rows.iter().map(|c| c.id),
                    ));
                    for row in rows {
                        if base
                            .channels
                            .get(row.id)
                            .is_some_and(|old| old.permissions != row.permissions)
                        {
                            self.await_channel(row.id);
                        }
                    }
                }
            }
            CacheResource::Roles => {
                self.unconfirmed_roles.extend(removed_roles(base, mutation));
                if let Some(rows) = &mutation.roles {
                    for row in rows {
                        if base
                            .roles
                            .get(row.id)
                            .is_some_and(|old| old.permissions != row.permissions)
                        {
                            self.changed_role_flags.insert(row.id);
                        }
                    }
                }
            }
            CacheResource::Members => {
                let incoming = mutation
                    .roster
                    .iter()
                    .flatten()
                    .chain(mutation.summaries.iter().flat_map(|batch| &batch.present))
                    .find(|user| user.id == own);
                if let Some(roles) = incoming.and_then(|user| user.roles.as_ref())
                    && let Some(old) = base.users.get(own).and_then(|user| user.roles.as_ref())
                {
                    self.own_assignments.extend(membership_changes(
                        old.iter().copied(),
                        roles.iter().copied(),
                    ));
                }
                self.observe_missing_peers(base, mutation);
            }
            CacheResource::Dms => {
                if let Some(rows) = &mutation.dms {
                    self.dm_membership.extend(membership_changes(
                        base.dms.ids(),
                        rows.iter().map(|(dm, _)| dm.id),
                    ));
                    for (dm, _) in rows {
                        if base
                            .dms
                            .get(dm.id)
                            .is_some_and(|old| old.peer_id != dm.peer_id)
                        {
                            self.dm_peers.insert(dm.id);
                        }
                    }
                }
            }
            CacheResource::Blocks | CacheResource::Profiles => {}
        }
    }

    pub(super) fn observe_missing_peers(
        &mut self,
        base: &ResourceSnapshot,
        mutation: &AdmittedMutation,
    ) {
        let absent: HashSet<_> = mutation
            .summaries
            .iter()
            .flat_map(|batch| &batch.absent)
            .copied()
            .collect();
        if !absent.is_empty() {
            self.missing_peers.extend(
                base.dms
                    .iter()
                    .filter(|dm| absent.contains(&dm.peer_id))
                    .map(|dm| dm.peer_id),
            );
        }
    }

    pub(crate) fn dm_unknown(&self, peer: Uuid) -> bool {
        self.missing_peers.contains(&peer)
    }

    pub(crate) fn unconfirmed_role(&self, role: Uuid) -> bool {
        self.unconfirmed_roles.contains(&role)
    }

    pub(crate) fn await_channel(&mut self, id: Uuid) {
        self.awaiting_channels.insert(id);
    }

    pub(super) fn recover(&mut self, mutation: &AdmittedMutation, own: Uuid) {
        use CacheResource::{Blocks, Channels, Dms, Members, Roles, Server};
        if mutation.complete {
            *self = Self::default();
            return;
        }
        let own_observed =
            mutation.roster.is_some() || mutation.summaries.iter().any(|batch| batch.covers(own));
        for (group, covered) in [
            (Server, mutation.server.is_some()),
            (Channels, mutation.channels.is_some()),
            (Roles, mutation.roles.is_some()),
            (Members, own_observed),
            (Dms, mutation.dms.is_some()),
            (Blocks, mutation.blocks.is_some()),
        ] {
            if covered {
                self.groups.remove(&group);
            }
        }
        if mutation.server.is_some() {
            self.owner_changed = false;
        }
        if own_observed {
            self.own_assignments.clear();
        }
        if mutation.dms.is_some() {
            self.dm_membership.clear();
            self.dm_peers.clear();
        }
        if mutation.dms.is_some() && mutation.blocks.is_some() {
            self.missing_peers.clear();
        }
        if mutation.roles.is_some() {
            self.unconfirmed_roles.clear();
            self.changed_role_flags.clear();
        }
        if mutation.channels.is_some() {
            self.awaiting_channels.clear();
            self.channel_membership.clear();
        }
    }

    pub(super) fn invalidate(&mut self, failure: &ScopedFailure) -> bool {
        failure.group.is_some_and(|group| self.groups.insert(group))
    }
}

/// Associates a malformed response with the endpoint that was actually requested.
pub(crate) struct ScopedFailure {
    pub(super) error: ClientError,
    group: Option<CacheResource>,
}

impl ScopedFailure {
    pub(crate) fn for_group(group: CacheResource, error: ClientError) -> Self {
        let malformed = matches!(
            error,
            ClientError::ResourceRejected(_) | ClientError::Api(papo_api::Error::Decode)
        );
        Self {
            error,
            group: (malformed && group != CacheResource::Profiles).then_some(group),
        }
    }

    /// A targeted fetch of another user's summary cannot supply the account's assignments.
    pub(crate) fn display(mut self) -> Self {
        self.group = None;
        self
    }

    pub(super) fn invalidates(&self) -> bool {
        self.group.is_some()
    }
}

impl From<ClientError> for ScopedFailure {
    fn from(error: ClientError) -> Self {
        Self { error, group: None }
    }
}

impl From<ResourceRejection> for ScopedFailure {
    fn from(error: ResourceRejection) -> Self {
        Self::for_group(error.group, error.into())
    }
}

/// Only an authoritative channel readback resolves a disappearing role's overwrite effects.
pub(crate) fn unresolved_overwrites(
    base: &ResourceSnapshot,
    mutation: &AdmittedMutation,
) -> OrdSet<Uuid> {
    if mutation.channels.is_some() {
        return OrdSet::new();
    }
    let removed = removed_roles(base, mutation);
    if removed.is_empty() {
        return OrdSet::new();
    }
    base.channels
        .iter()
        .filter(|channel| {
            channel
                .permissions
                .iter()
                .any(|overwrite| removed.contains(&overwrite.key.parsed()))
        })
        .map(|channel| channel.id)
        .collect()
}

pub(crate) fn removed_roles(base: &ResourceSnapshot, mutation: &AdmittedMutation) -> OrdSet<Uuid> {
    let Some(roles) = &mutation.roles else {
        return OrdSet::new();
    };
    let current: HashSet<_> = roles.iter().map(|role| role.id).collect();
    base.roles
        .iter()
        .map(|role| role.id)
        .filter(|id| !current.contains(id))
        .collect()
}

fn membership_changes(
    old: impl Iterator<Item = Uuid>,
    next: impl Iterator<Item = Uuid>,
) -> Vec<Uuid> {
    let old: HashSet<_> = old.collect();
    let next: HashSet<_> = next.collect();
    old.symmetric_difference(&next).copied().collect()
}
