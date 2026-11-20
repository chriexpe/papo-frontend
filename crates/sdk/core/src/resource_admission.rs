//! Wire admission for authoritative resource replacements. Fetching, ordering, epochs,
//! durable publication and access decisions stay with the resource commit path and R1.
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
//! | `channels` list absent/null, or longer than the backend's 500-channel maximum | security | rejects Channels |
//! | channel `id`, `type` (`text`, `category` or `voice`), `permissions` (absent is not `[]`), duplicate channel | security | rejects Channels |
//! | overwrite `role_id` (any UUID spelling, kept verbatim), `permissions.read_channel`, repeated key | security | rejects Channels |
//! | `roles` list absent/null; role `id`, `permissions`; duplicate role | security | rejects Roles |
//! | member page `users`, `has_more`, page boundary, a cursor that does not strictly advance | security | rejects Members |
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
//! A database that holds more than 500 non-DM channels (only possible outside the
//! backend's creation path) is listed as exactly 500 rows, indistinguishable from a full
//! server. That is a limit of the backend's completeness contract, not something
//! admission can detect.
//!
//! Validation here does not replace the account cache's encoder checks; those remain a
//! disk-integrity guard and should never fire for admitted data.

mod channels;
mod people;
mod relationships;
#[cfg(test)]
mod tests;

use crate::{CacheResource, Uuid};
use std::sync::Mutex;

pub(crate) use people::{Roster, RosterPage};

/// The pinned backend refuses to create a channel once a server has this many, and
/// `GET /channels` returns at most this many rows. A list of exactly this length is a
/// legitimate full server. A longer one cannot come from the backend.
const MAX_CHANNELS: usize = 500;

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
    /// The list cannot prove it is complete: a page boundary did not advance.
    Incomplete,
    /// A list was longer than the backend's own maximum.
    ExceedsLimit,
    /// A value outside the set the pinned backend can send, such as an unknown channel
    /// type; it is never read as an ordinary, open channel.
    UnsupportedValue,
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

    pub fn for_disk(own: Uuid, diagnostics: &'a Diagnostics) -> Self {
        Self { own, diagnostics }
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
/// PostgreSQL UUID columns serialize canonically; overwrite object keys are parsed separately.
fn parse_id(value: Option<&str>) -> Option<Uuid> {
    let value = value?;
    let id = Uuid::try_parse(value).ok()?;
    (id.hyphenated().encode_lower(&mut Uuid::encode_buffer()) == value).then_some(id)
}
