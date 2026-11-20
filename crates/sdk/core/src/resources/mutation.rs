//! Pure transitions from admitted facts. Producers own fetch sequencing, commits own side effects.

use super::{
    ChannelRecord, DirectRecord, ProfileRecord, ResourceSnapshot, RoleRecord, ServerRecord,
    UserObservation, UserRecord,
};
use crate::{AdmissionFault, CacheResource, ResourceRejection, Uuid};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// Targeted member results. Unresolved identities occur in neither collection.
#[derive(Default)]
pub(crate) struct SummaryBatch {
    pub present: Vec<UserObservation>,
    pub absent: Vec<Uuid>,
}
impl SummaryBatch {
    pub fn covers(&self, id: Uuid) -> bool {
        self.present.iter().any(|user| user.id == id) || self.absent.contains(&id)
    }
}

#[derive(Default)]
pub(crate) struct ProfileBatch {
    pub present: Vec<(ProfileRecord, UserObservation)>,
    pub absent: Vec<Uuid>,
}
impl ProfileBatch {
    pub fn single(row: (ProfileRecord, UserObservation)) -> Self {
        Self {
            present: vec![row],
            absent: Vec::new(),
        }
    }
}

/// A batch of admitted observations. An omitted group leaves its previous facts intact.
#[derive(Default)]
pub(crate) struct AdmittedMutation {
    pub complete: bool,
    pub server: Option<ServerRecord>,
    pub channels: Option<Vec<ChannelRecord>>,
    pub roles: Option<Vec<RoleRecord>>,
    pub roster: Option<Vec<UserObservation>>,
    pub summaries: Vec<SummaryBatch>,
    pub profiles: Vec<ProfileBatch>,
    pub dms: Option<Vec<(DirectRecord, UserObservation)>>,
    pub blocks: Option<Vec<UserObservation>>,
    pub invalidate_profiles: Vec<Uuid>,
}

impl AdmittedMutation {
    fn validate_complete(&self) -> Result<(), ResourceRejection> {
        if self.complete {
            for (group, present) in [
                (CacheResource::Server, self.server.is_some()),
                (CacheResource::Channels, self.channels.is_some()),
                (CacheResource::Roles, self.roles.is_some()),
                (CacheResource::Members, self.roster.is_some()),
                (CacheResource::Dms, self.dms.is_some()),
                (CacheResource::Blocks, self.blocks.is_some()),
            ] {
                if !present {
                    return Err(ResourceRejection {
                        group,
                        fault: AdmissionFault::Incomplete,
                    });
                }
            }
        }
        Ok(())
    }

    pub fn apply(
        self,
        base: &ResourceSnapshot,
        own: Uuid,
    ) -> Result<ResourceSnapshot, ResourceRejection> {
        self.validate_complete()?;
        let mut next = base.clone();
        let mut orphan_candidates = HashSet::new();
        if self.complete {
            next.channel_census_complete = true;
            orphan_candidates.extend(next.profiles.ids());
            next.profiles = super::ResourceTable::default();
        }
        if let Some(server) = self.server
            && next.server.as_deref() != Some(&server)
        {
            next.server = Some(Arc::new(server));
        }
        if let Some(channels) = self.channels {
            next.channels.replace(channels, |row| row.id);
        }
        let mut deleted_roles = HashSet::new();
        let definitions_observed = self.roles.is_some();
        if let Some(roles) = self.roles {
            next.roles.replace(roles, |row| row.id);
            deleted_roles.extend(
                next.roles
                    .diff_keys(&base.roles)
                    .filter(|id| next.roles.get(*id).is_none()),
            );
        }
        let fresh_own = self.roster.iter().flatten().any(|row| row.id == own)
            || self
                .summaries
                .iter()
                .any(|batch| batch.present.iter().any(|user| user.id == own));
        let mut people = HashMap::new();
        let mut refreshed_profiles = HashSet::new();
        if let Some(blocks) = self.blocks {
            orphan_candidates.extend(next.blocks.iter());
            next.blocks.replace(blocks.iter().map(|user| user.id));
            for user in blocks {
                user.sparse(base, own).overlay(&mut people);
            }
        }
        if let Some(dms) = self.dms {
            orphan_candidates.extend(next.dms.iter().map(|dm| dm.peer_id));
            let records = dms
                .into_iter()
                .map(|(dm, user)| {
                    user.sparse(base, own).overlay(&mut people);
                    dm
                })
                .collect();
            next.dms.replace(records, |dm| dm.id);
        }
        for batch in self.profiles {
            for id in batch.absent {
                next.profiles.remove(id);
                orphan_candidates.insert(id);
            }
            for (detail, user) in batch.present {
                user.sparse(base, own).overlay(&mut people);
                refreshed_profiles.insert(detail.id);
                next.profiles.set(detail.id, detail);
            }
        }
        let additions = next.update_members(
            self.roster,
            self.summaries,
            &mut orphan_candidates,
            &mut people,
        );
        if definitions_observed
            && fresh_own
            && people
                .get(&own)
                .and_then(|user| user.roles.as_ref())
                .is_some_and(|roles| roles.iter().any(|id| next.roles.get(*id).is_none()))
        {
            return Err(ResourceRejection {
                group: CacheResource::Members,
                fault: AdmissionFault::UnexpectedId,
            });
        }
        next.install_user_observations(people, &deleted_roles, &refreshed_profiles);
        for id in additions {
            if !next.members.contains(id)
                && let Some(user) = next.users.get(id)
            {
                next.members.insert(next.insertion_position(user), id);
            }
        }
        next.remove_deleted_assignments(&deleted_roles);
        for id in self.invalidate_profiles {
            if !refreshed_profiles.contains(&id) {
                next.profiles.remove(id);
                orphan_candidates.insert(id);
            }
        }
        next.remove_orphans(orphan_candidates);
        Ok(next)
    }
}

impl ResourceSnapshot {
    fn update_members(
        &mut self,
        roster: Option<Vec<UserObservation>>,
        summaries: Vec<SummaryBatch>,
        orphan_candidates: &mut HashSet<Uuid>,
        people: &mut HashMap<Uuid, UserObservation>,
    ) -> Vec<Uuid> {
        if let Some(roster) = roster {
            orphan_candidates.extend(self.members.iter());
            self.members.replace(roster.iter().map(|row| row.id));
            for user in roster {
                user.overlay(people);
            }
        }
        let mut additions = Vec::new();
        for batch in summaries {
            for user in batch.present {
                if !self.members.contains(user.id) {
                    additions.push(user.id);
                }
                user.overlay(people);
            }
            for id in batch.absent {
                people.remove(&id);
                self.members.remove(id);
                self.users.remove(id);
                self.profiles.remove(id);
            }
        }
        additions
    }

    fn install_user_observations(
        &mut self,
        people: HashMap<Uuid, UserObservation>,
        deleted_roles: &HashSet<Uuid>,
        refreshed_profiles: &HashSet<Uuid>,
    ) {
        for (id, observation) in people {
            let mut user = observation.resolve(self.users.get(id));
            if !deleted_roles.is_empty()
                && let Some(roles) = &mut user.roles
            {
                roles.retain(|id| !deleted_roles.contains(id));
            }
            self.member_summary(user, refreshed_profiles);
        }
    }

    fn member_summary(&mut self, user: UserRecord, refreshed_profiles: &HashSet<Uuid>) {
        if !refreshed_profiles.contains(&user.id)
            && self.users.get(user.id).is_some_and(|old| {
                old.username != user.username
                    || old.nickname != user.nickname
                    || old.typing != user.typing
            })
        {
            self.profiles.remove(user.id);
        }
        self.users.set(user.id, user);
    }

    fn insertion_position(&self, user: &UserRecord) -> usize {
        let key = |user: &UserRecord| {
            user.created_at
                .as_deref()
                .and_then(|value| crate::parse_message_timestamp_us(value).ok())
                .map(|time| (time, user.id))
        };
        let Some(new) = key(user) else {
            return self.members.len();
        };
        self.members
            .iter()
            .position(|id| self.users.get(id).and_then(key).is_none_or(|old| old > new))
            .unwrap_or(self.members.len())
    }

    fn remove_deleted_assignments(&mut self, deleted: &HashSet<Uuid>) {
        if deleted.is_empty() {
            return;
        }
        let affected: Vec<_> = self
            .users
            .ids()
            .filter(|id| {
                self.users
                    .get(*id)
                    .and_then(|user| user.roles.as_ref())
                    .is_some_and(|roles| roles.iter().any(|role| deleted.contains(role)))
            })
            .collect();
        for id in affected {
            if let Some(mut user) = self.users.get(id).cloned() {
                if let Some(roles) = &mut user.roles {
                    roles.retain(|role| !deleted.contains(role));
                }
                self.users.set(id, user);
            }
        }
    }

    fn remove_orphans(&mut self, mut candidates: HashSet<Uuid>) {
        candidates.retain(|id| {
            !self.members.contains(*id)
                && !self.blocks.contains(*id)
                && self.profiles.get(*id).is_none()
        });
        if candidates.is_empty() {
            return;
        }
        for dm in self.dms.iter() {
            candidates.remove(&dm.peer_id);
        }
        for id in candidates {
            self.users.remove(id);
        }
    }

    /// Groups whose persisted rows differ, including joined user summaries in relationship rows.
    pub(crate) fn changed_groups(&self, old: &Self) -> Vec<CacheResource> {
        let mut changed = Vec::new();
        let users: HashSet<_> = self.users.diff_keys(&old.users).collect();
        if self.server != old.server {
            changed.push(CacheResource::Server);
        }
        if self.channels != old.channels {
            changed.push(CacheResource::Channels);
        }
        if self.roles != old.roles {
            changed.push(CacheResource::Roles);
        }
        if self.members != old.members || users.iter().any(|id| self.members.contains(*id)) {
            changed.push(CacheResource::Members);
        }
        if self.profiles != old.profiles || users.iter().any(|id| self.profiles.get(*id).is_some())
        {
            changed.push(CacheResource::Profiles);
        }
        if self.dms != old.dms
            || (!users.is_empty() && self.dms.iter().any(|dm| users.contains(&dm.peer_id)))
        {
            changed.push(CacheResource::Dms);
        }
        if self.blocks != old.blocks || users.iter().any(|id| self.blocks.contains(*id)) {
            changed.push(CacheResource::Blocks);
        }
        changed
    }
}
