//! Admission of member rosters, summary batches, DMs and blocks; server, channel and
//! role definitions live in the sibling `channels` module.

use super::{Admission, AdmissionFault, ResourceRejection, parse_id};
use crate::{CacheResource, Uuid};
use papo_api::wire::{
    BlockListResponse, DirectConversation, DirectConversationList, MemberListResponse, RoleSummary,
    UserProfile, UserSummary,
};
use std::collections::{HashMap, HashSet};

/// Where the next `GET /users` page starts: the previous page's last `(created_at, id)`.
pub(crate) type PageCursor = (String, String);

/// Result of admitting one roster page.
pub(crate) enum RosterPage {
    /// The backend has more rows; request the page after this cursor.
    More(PageCursor),
    /// The backend proved the roster complete; this is the admitted replacement.
    Complete(Vec<UserSummary>),
}

/// Accumulates a paginated roster. Nothing is admitted until a page proves completion.
pub(crate) struct Roster<'a> {
    admission: Admission<'a>,
    users: Vec<UserSummary>,
    /// The previous boundary as the backend orders it: `(created_at, id)`, with the
    /// timestamp in microseconds and the UUID in byte order, like Postgres.
    last: Option<(i64, Uuid)>,
}

impl<'a> Roster<'a> {
    pub fn new(admission: Admission<'a>) -> Self {
        Self {
            admission,
            users: Vec::new(),
            last: None,
        }
    }

    /// Admits one page. A page without `users` or `has_more` proves nothing, so it is
    /// rejected rather than read as the end of the roster.
    /// # Errors
    /// Rejects `Members` for a missing list or flag, a "more" page without a usable
    /// boundary, a boundary that does not come strictly after the previous one, or a
    /// malformed complete roster.
    pub fn page(&mut self, page: MemberListResponse) -> Result<RosterPage, ResourceRejection> {
        let reject = |fault| self.admission.reject(CacheResource::Members, fault);
        let users = page
            .users
            .ok_or_else(|| reject(AdmissionFault::MissingList))?;
        let has_more = page
            .has_more
            .ok_or_else(|| reject(AdmissionFault::MissingList))?;
        if !has_more {
            self.users.extend(users);
            let users = std::mem::take(&mut self.users);
            return self
                .admission
                .members(users, Scope::Roster)
                .map(RosterPage::Complete);
        }
        let (cursor, position) = users
            .last()
            .and_then(boundary)
            .ok_or_else(|| reject(AdmissionFault::Incomplete))?;
        // The backend pages by `(created_at, id) > cursor`; a boundary that does not move
        // forward would repeat or skip rows, so the roster could never prove completion.
        if self.last.is_some_and(|last| position <= last) {
            return Err(reject(AdmissionFault::Incomplete));
        }
        self.last = Some(position);
        self.users.extend(users);
        Ok(RosterPage::More(cursor))
    }
}

/// The cursor text sent back to the backend, and its position in the backend's order.
fn boundary(last: &UserSummary) -> Option<(PageCursor, (i64, Uuid))> {
    let created_at = last.created_at.clone()?;
    let raw_id = last.id.clone()?;
    let id = parse_id(Some(&raw_id))?;
    let created_at_us = crate::parse_message_timestamp_us(&created_at).ok()?;
    Some(((created_at, raw_id), (created_at_us, id)))
}

/// Whether a member list must contain the account's own row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    /// The complete roster: the account is always a member, so its absence is malformed.
    Roster,
    /// A targeted batch, which legitimately omits users that do not exist.
    Batch,
}

impl Admission<'_> {
    /// Admits a `user_summary_batch` response for `requested`. An omitted user does not
    /// exist (the backend skips unknown IDs). Another member's unparseable, unrequested
    /// or repeated row is dropped like any display-only row; the own row is
    /// security-bearing.
    /// # Errors
    /// Rejects `Members` for an omitted, unrequested, repeated or malformed own row.
    pub fn summaries(
        self,
        requested: &[Uuid],
        response: Vec<UserSummary>,
    ) -> Result<Vec<UserSummary>, ResourceRejection> {
        let mut attributable = Vec::with_capacity(response.len());
        for user in response {
            match parse_id(user.id.as_deref()) {
                None => self.drop_row(CacheResource::Members, AdmissionFault::InvalidId),
                Some(id) if id == self.own && !requested.contains(&id) => {
                    return Err(self.reject(CacheResource::Members, AdmissionFault::UnexpectedId));
                }
                Some(id) if !requested.contains(&id) => {
                    self.drop_row(CacheResource::Members, AdmissionFault::UnexpectedId);
                }
                Some(_) => attributable.push(user),
            }
        }
        // The signed-in account exists, so omitting its requested row is not "no such user".
        let own = self.own.to_string();
        let own_returned = attributable
            .iter()
            .any(|user| user.id.as_deref() == Some(own.as_str()));
        if requested.contains(&self.own) && !own_returned {
            return Err(self.reject(CacheResource::Members, AdmissionFault::MissingOwnRow));
        }
        self.members(attributable, Scope::Batch)
    }

    /// Admits `GET /users/{id}/profile` for exactly `requested`.
    /// # Errors
    /// Rejects `Profiles` for a profile naming another or a non-canonical identity.
    pub fn profile(
        self,
        requested: Uuid,
        profile: UserProfile,
    ) -> Result<UserProfile, ResourceRejection> {
        if parse_id(profile.id.as_deref()) != Some(requested) {
            return Err(self.reject(CacheResource::Profiles, AdmissionFault::UnexpectedId));
        }
        Ok(profile)
    }

    /// Admits the user a member event names, in any UUID spelling; requests and merges
    /// then use its canonical form.
    /// # Errors
    /// Rejects `Members` for an ID that is not a UUID.
    pub fn event_user(self, raw: &str) -> Result<Uuid, ResourceRejection> {
        raw.parse()
            .map_err(|_| self.reject(CacheResource::Members, AdmissionFault::InvalidId))
    }

    /// Admits the complete `GET /dms` replacement.
    /// # Errors
    /// Rejects `Dms` for a conversation without a unique ID or an attributable peer.
    pub fn dms(
        self,
        list: DirectConversationList,
    ) -> Result<Vec<DirectConversation>, ResourceRejection> {
        let mut seen = HashSet::with_capacity(list.dms.len());
        for dm in &list.dms {
            let Some(id) = parse_id(Some(&dm.id)) else {
                return Err(self.reject(CacheResource::Dms, AdmissionFault::InvalidId));
            };
            if !seen.insert(id) {
                return Err(self.reject(CacheResource::Dms, AdmissionFault::DuplicateId));
            }
            if parse_id(dm.user.id.as_deref()).is_none() {
                return Err(self.reject(CacheResource::Dms, AdmissionFault::InvalidId));
            }
        }
        Ok(list.dms)
    }

    /// Admits the complete `GET /users/blocks` replacement.
    /// # Errors
    /// Rejects `Blocks` for an entry without a unique ID.
    pub fn blocks(self, list: BlockListResponse) -> Result<Vec<UserSummary>, ResourceRejection> {
        let mut seen = HashSet::with_capacity(list.users.len());
        for user in &list.users {
            let Some(id) = parse_id(user.id.as_deref()) else {
                return Err(self.reject(CacheResource::Blocks, AdmissionFault::InvalidId));
            };
            if !seen.insert(id) {
                return Err(self.reject(CacheResource::Blocks, AdmissionFault::DuplicateId));
            }
        }
        Ok(list.users)
    }

    /// The account's own row is security-bearing and rejects the group; another
    /// member's malformed row or role entry is dropped with a diagnostic.
    fn members(
        self,
        users: Vec<UserSummary>,
        scope: Scope,
    ) -> Result<Vec<UserSummary>, ResourceRejection> {
        let reject = |fault| self.reject(CacheResource::Members, fault);
        // Count first: a repeated ID is ambiguous for every copy, not only the later ones.
        let mut counts: HashMap<Uuid, usize> = HashMap::with_capacity(users.len());
        for user in &users {
            if let Some(id) = parse_id(user.id.as_deref()) {
                *counts.entry(id).or_default() += 1;
            }
        }
        match counts.get(&self.own) {
            Some(1) => {}
            Some(_) => return Err(reject(AdmissionFault::DuplicateId)),
            None if scope == Scope::Roster => return Err(reject(AdmissionFault::MissingOwnRow)),
            None => {}
        }
        let mut admitted = Vec::with_capacity(users.len());
        for mut user in users {
            let Some(id) = parse_id(user.id.as_deref()) else {
                self.drop_row(CacheResource::Members, AdmissionFault::InvalidId);
                continue;
            };
            if counts.get(&id).copied() != Some(1) {
                self.drop_row(CacheResource::Members, AdmissionFault::DuplicateId);
                continue;
            }
            if id == self.own {
                own_roles_fault(&user).map_or(Ok(()), |fault| Err(reject(fault)))?;
            } else {
                self.strip_display_roles(&mut user);
            }
            admitted.push(user);
        }
        Ok(admitted)
    }

    /// Removes another member's unattributable or repeated role labels; display only.
    fn strip_display_roles(self, user: &mut UserSummary) {
        let Some(roles) = user.roles.as_mut() else {
            return;
        };
        let mut counts: HashMap<Uuid, usize> = HashMap::with_capacity(roles.len());
        for role in roles.iter() {
            if let Some(id) = parse_id(role.id.as_deref()) {
                *counts.entry(id).or_default() += 1;
            }
        }
        roles.retain(|role| match parse_id(role.id.as_deref()) {
            None => {
                self.drop_row(CacheResource::Members, AdmissionFault::InvalidId);
                false
            }
            Some(id) if counts.get(&id).copied() != Some(1) => {
                self.drop_row(CacheResource::Members, AdmissionFault::DuplicateId);
                false
            }
            Some(_) => true,
        });
    }
}

/// The evaluator reads these assignments; a missing or ambiguous one is not "no role".
fn own_roles_fault(user: &UserSummary) -> Option<AdmissionFault> {
    let Some(roles) = &user.roles else {
        return Some(AdmissionFault::MissingField);
    };
    let mut seen = HashSet::with_capacity(roles.len());
    roles.iter().find_map(|role: &RoleSummary| {
        let Some(id) = parse_id(role.id.as_deref()) else {
            return Some(AdmissionFault::InvalidId);
        };
        (!seen.insert(id)).then_some(AdmissionFault::DuplicateId)
    })
}
