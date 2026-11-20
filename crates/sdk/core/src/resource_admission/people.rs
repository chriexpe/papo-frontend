//! Admission of member rosters, summary batches, DMs and blocks; server, channel and
//! role definitions live in the sibling `channels` module.

use super::{Admission, AdmissionFault, ResourceRejection, parse_id};
use crate::resources::{SummaryBatch, UserObservation};
use crate::{CacheResource, Uuid};
use papo_api::wire::{MemberListResponse, RoleSummary, UserSummary};
use std::collections::{HashMap, HashSet};

/// Where the next `GET /users` page starts: the previous page's last `(created_at, id)`.
pub(crate) type PageCursor = (String, String);

/// Result of admitting one roster page.
pub(crate) enum RosterPage {
    /// The backend has more rows; request the page after this cursor.
    More(PageCursor),
    /// The backend proved the roster complete; this is the admitted replacement.
    Complete(Vec<UserObservation>),
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
            return self.admission.roster(users).map(RosterPage::Complete);
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

impl Admission<'_> {
    /// Complete member pages must include the authenticated user.
    pub fn roster(
        self,
        users: Vec<UserSummary>,
    ) -> Result<Vec<UserObservation>, ResourceRejection> {
        Ok(self.member_rows(None, users, true)?.present)
    }

    /// Cache rows can hold a partial roster. An own row, if present, still requires valid assignments.
    pub fn cached_members(
        self,
        users: Vec<UserSummary>,
    ) -> Result<Vec<UserObservation>, ResourceRejection> {
        Ok(self.member_rows(None, users, false)?.present)
    }

    /// Rows dropped for ambiguity are never interpreted as authoritative omissions.
    pub fn summaries(
        self,
        requested: &[Uuid],
        users: Vec<UserSummary>,
    ) -> Result<SummaryBatch, ResourceRejection> {
        self.member_rows(Some(requested), users, requested.contains(&self.own))
    }

    fn member_rows(
        self,
        requested: Option<&[Uuid]>,
        users: Vec<UserSummary>,
        require_own: bool,
    ) -> Result<SummaryBatch, ResourceRejection> {
        let group = CacheResource::Members;
        let mut seen = HashSet::new();
        let mut duplicates = HashSet::new();
        let mut unattributable = false;
        let mut parsed = Vec::with_capacity(users.len());
        for user in users {
            let Some(id) = parse_id(user.id.as_deref()) else {
                self.drop_row(group, AdmissionFault::InvalidId);
                unattributable = true;
                continue;
            };
            if requested.is_some_and(|ids| !ids.contains(&id)) {
                if id == self.own {
                    return Err(self.reject(group, AdmissionFault::UnexpectedId));
                }
                self.drop_row(group, AdmissionFault::UnexpectedId);
                continue;
            }
            if !seen.insert(id) {
                duplicates.insert(id);
            }
            parsed.push((id, user));
        }
        if require_own && !seen.contains(&self.own) {
            return Err(self.reject(group, AdmissionFault::MissingOwnRow));
        }
        if duplicates.contains(&self.own) {
            return Err(self.reject(group, AdmissionFault::DuplicateId));
        }
        let mut present = Vec::with_capacity(parsed.len());
        for (id, user) in parsed {
            if duplicates.contains(&id) {
                self.drop_row(group, AdmissionFault::DuplicateId);
                continue;
            }
            if id == self.own && user.roles.is_none() {
                return Err(self.reject(group, AdmissionFault::MissingField));
            }
            present.push(self.user(group, id, user, id == self.own)?);
        }
        let absent = requested
            .into_iter()
            .flatten()
            .copied()
            .filter(|id| !unattributable && !seen.contains(id))
            .collect();
        Ok(SummaryBatch { present, absent })
    }

    pub(super) fn user(
        self,
        group: CacheResource,
        id: Uuid,
        user: UserSummary,
        strict: bool,
    ) -> Result<UserObservation, ResourceRejection> {
        Ok(UserObservation {
            id,
            username: user.username,
            nickname: user.nickname,
            banned: user.banned,
            created_at: user.created_at,
            typing: user.typing,
            roles: user
                .roles
                .map(|roles| self.assignments(group, roles, strict))
                .transpose()?,
        })
    }

    fn assignments(
        self,
        group: CacheResource,
        roles: Vec<RoleSummary>,
        strict: bool,
    ) -> Result<Vec<Uuid>, ResourceRejection> {
        let mut counts = HashMap::with_capacity(roles.len());
        let mut ids = Vec::with_capacity(roles.len());
        for role in roles {
            if let Some(id) = parse_id(role.id.as_deref()) {
                *counts.entry(id).or_insert(0_usize) += 1;
                ids.push(id);
            } else if strict {
                return Err(self.reject(group, AdmissionFault::InvalidId));
            } else {
                self.drop_row(group, AdmissionFault::InvalidId);
            }
        }
        if strict && counts.values().any(|count| *count > 1) {
            return Err(self.reject(group, AdmissionFault::DuplicateId));
        }
        ids.retain(|id| {
            if counts[id] == 1 {
                return true;
            }
            self.drop_row(group, AdmissionFault::DuplicateId);
            false
        });
        Ok(ids)
    }

    pub fn event_user(self, raw: &str) -> Result<Uuid, ResourceRejection> {
        raw.parse()
            .map_err(|_| self.reject(CacheResource::Members, AdmissionFault::InvalidId))
    }
}
