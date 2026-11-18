//! Wire admission for authoritative resource replacements. Fetching, ordering, epochs,
//! cache writes and access decisions stay with the reconnect, gateway and
//! operation-refresh producers, which all call this module.
//!
//! A producer passes every fetched group through here before it may replace any part
//! of the account's resource image. Each group is admitted whole or rejected whole. A
//! rejected group keeps its last admitted value and cache rows, and its
//! [`ResourceRejection`] names the group, so a consumer can treat the facts that depend
//! on it as unknown. A rejection is a wire fault, never cache corruption. An explicit
//! empty list is a real empty replacement. An absent or null list is a rejection.
//!
//! Classification (security-bearing data rejects its group; display-only data is
//! dropped with a [`DroppedRow`] diagnostic and the rest is admitted):
//!
//! | Data | Class | Malformed |
//! | --- | --- | --- |
//! | server `id`; `owner_id` when present (null means no owner) | security | rejects Server |
//! | `channels` list absent/null, or at the backend's 500-row cap | security | rejects Channels |
//! | channel `id`, `type`, `permissions` (absent is not `[]`), duplicate channel | security | rejects Channels |
//! | overwrite `role_id` (any UUID spelling, kept verbatim), `permissions.read_channel`, repeated key | security | rejects Channels |
//! | `roles` list absent/null; role `id`, `permissions`; duplicate role | security | rejects Roles |
//! | member page `users`, `has_more`, page boundary, repeated cursor | security | rejects Members |
//! | own member row: present in a complete roster, unique, `roles` present, role IDs | security | rejects Members |
//! | summary batch: own row omitted, repeated or unrequested | security | rejects Members |
//! | summary batch: another row unparseable, unrequested or repeated | display | row dropped |
//! | requested profile with another identity | display group | rejects Profiles |
//! | another member's row with an unparseable or repeated ID | display | row dropped |
//! | another member's role entry with an unparseable or repeated ID | display | entry dropped |
//! | `dms[].id`, `dms[].user.id`, duplicate DM | security | rejects Dms |
//! | `blocks[].id`, duplicate block | security | rejects Blocks |
//! | names, colors, topics, positions and other cosmetic fields | display | admitted unchanged |
//!
//! Every identity above except an overwrite's `role_id` must be a UUID in canonical
//! lowercase hyphenated form: the backend emits those from uuid columns.
//!
//! Why the split: channel overwrites and the account's own role assignments decide what
//! this account may read, so dropping one entry can open a channel (a restrictive
//! overwrite removed from a one-entry list leaves an open channel) or close one (a
//! granting overwrite or own role removed). Another member's row and labels feed
//! display only: the evaluator reads the account's own assignments, and role grants
//! only add, so dropping someone else's data never widens this account's access.
//!
//! Validation here does not replace the account cache's encoder checks; those remain a
//! disk-integrity guard and should never fire for admitted data.

mod channels;
mod people;
#[cfg(test)]
mod tests;

use crate::{CacheResource, Uuid};
use std::sync::Mutex;

pub(crate) use people::{Roster, RosterPage};

/// Rows the pinned backend returns at most from `GET /channels`; a list that reaches
/// it cannot prove that an unlisted channel was removed.
const CHANNEL_LIST_CAP: usize = 500;

/// Why a wire response, or one of its rows, was not admitted. Carries no identifiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AdmissionFault {
    /// A required list or completion flag was absent or null.
    MissingList,
    /// A required security-bearing field was absent or null.
    MissingField,
    /// An identity was empty or not a UUID.
    InvalidId,
    /// Two entries claimed the same identity.
    DuplicateId,
    /// A targeted response named an identity that was not requested.
    UnexpectedId,
    /// The account's own row was absent from a complete roster.
    MissingOwnRow,
    /// The list cannot prove it is complete: a page boundary did not advance, or the
    /// list reached a backend row cap.
    Incomplete,
}

/// An authoritative replacement that was not applied. The group keeps its last admitted
/// value; the error is a wire fault and never invalidates cached data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("resource response rejected")]
#[non_exhaustive]
pub struct ResourceRejection {
    /// The resource group whose replacement was rejected.
    pub group: CacheResource,
    /// What made the response inadmissible.
    pub fault: AdmissionFault,
}

/// A display-only row or entry left out of an otherwise admitted replacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DroppedRow {
    /// The group the row belonged to.
    pub group: CacheResource,
    /// What was malformed about it.
    pub fault: AdmissionFault,
}

/// Session-scoped admission counts, for diagnostics only. No identifiers or server text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct AdmissionDiagnostics {
    /// Display-only rows or entries dropped from admitted replacements.
    pub dropped_rows: u64,
    /// The most recently dropped row.
    pub last_dropped: Option<DroppedRow>,
    /// Authoritative replacements rejected.
    pub rejected_replacements: u64,
    /// The most recent rejection.
    pub last_rejection: Option<ResourceRejection>,
}

/// One session's diagnostics sink. Its lock is never held across an await.
#[derive(Default)]
pub(crate) struct Diagnostics(Mutex<AdmissionDiagnostics>);

impl Diagnostics {
    pub fn snapshot(&self) -> AdmissionDiagnostics {
        *self.lock()
    }

    fn dropped(&self, group: CacheResource, fault: AdmissionFault) {
        let mut diagnostics = self.lock();
        diagnostics.dropped_rows = diagnostics.dropped_rows.saturating_add(1);
        diagnostics.last_dropped = Some(DroppedRow { group, fault });
    }

    fn rejected(&self, rejection: ResourceRejection) {
        let mut diagnostics = self.lock();
        diagnostics.rejected_replacements = diagnostics.rejected_replacements.saturating_add(1);
        diagnostics.last_rejection = Some(rejection);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, AdmissionDiagnostics> {
        // Counters stay meaningful after a panicking writer; there is no invariant to lose.
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Admission context for one account: whose rows are security-bearing, and where
/// diagnostics go. Producers build one per fetch from their session.
#[derive(Clone, Copy)]
pub(crate) struct Admission<'a> {
    own: Uuid,
    diagnostics: &'a Diagnostics,
}

impl<'a> Admission<'a> {
    pub fn new(session: &'a crate::session::SessionInner) -> Self {
        Self {
            own: session.account.user(),
            diagnostics: &session.admission,
        }
    }

    #[cfg(test)]
    pub fn for_test(own: Uuid, diagnostics: &'a Diagnostics) -> Self {
        Self { own, diagnostics }
    }

    fn reject(self, group: CacheResource, fault: AdmissionFault) -> ResourceRejection {
        let rejection = ResourceRejection { group, fault };
        self.diagnostics.rejected(rejection);
        rejection
    }

    fn drop_row(self, group: CacheResource, fault: AdmissionFault) {
        self.diagnostics.dropped(group, fault);
    }
}

/// Parses a required identity in the backend's canonical lowercase hyphenated form.
/// Consumers still compare these IDs as strings (the evaluator matches overwrite and
/// own role IDs textually), so a non-canonical spelling of a valid UUID would silently
/// miss a grant. It is invalid here instead.
fn parse_id(value: Option<&str>) -> Option<Uuid> {
    let value = value?;
    let id = Uuid::try_parse(value).ok()?;
    (id.hyphenated().to_string() == value).then_some(id)
}
