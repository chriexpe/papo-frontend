//! Immutable account resource facts. Access decisions, transport and durable cache revisions have separate owners.

mod mutation;
mod records;
mod tables;
mod user_observation;
use crate::{Generation, resource_commit::EvidenceGaps};
pub(crate) use mutation::{AdmittedMutation, ProfileBatch, SummaryBatch};
pub use records::*;
use std::sync::Arc;
pub use tables::{EntityTable, ResourceIds, ResourceTable};
pub(crate) use user_observation::UserObservation;

/// Coherent resource image for a single authenticated account.
#[derive(Clone, Debug)]
pub struct ResourceSnapshot {
    /// Public observation version, scoped to the session.
    pub version: u64,
    /// Connection generation.
    pub generation: Generation,
    /// A full live replacement has established this generation's channel census.
    pub channel_census_complete: bool,
    /// Server metadata without image bytes.
    pub server: Option<Arc<ServerRecord>>,
    /// Ordinary channels in backend order.
    pub channels: ResourceTable<ChannelRecord>,
    /// Current role definitions.
    pub roles: ResourceTable<RoleRecord>,
    /// User summaries shared by all relationships.
    pub users: EntityTable<UserRecord>,
    /// Ordered roster membership.
    pub members: ResourceIds,
    /// Requested profile details.
    pub profiles: ResourceTable<ProfileRecord>,
    /// Direct conversations in backend order.
    pub dms: ResourceTable<DirectRecord>,
    /// Ordered block relationships.
    pub blocks: ResourceIds,
    // Rejected observations can publish gaps before the first cache load.
    pub(crate) has_display_baseline: bool,
    pub(crate) gaps: EvidenceGaps,
}

impl ResourceSnapshot {
    /// An unverified empty image.
    #[must_use]
    pub fn empty(generation: Generation) -> Self {
        Self {
            version: 0,
            generation,
            channel_census_complete: false,
            server: None,
            channels: ResourceTable::default(),
            roles: ResourceTable::default(),
            users: EntityTable::default(),
            members: ResourceIds::default(),
            profiles: ResourceTable::default(),
            dms: ResourceTable::default(),
            blocks: ResourceIds::default(),
            has_display_baseline: false,
            gaps: EvidenceGaps::default(),
        }
    }
}

/// Wire rows for a snapshot built outside a session: a test fixture or a fictional preview.
#[derive(Clone, Default)]
pub struct SnapshotRows {
    /// The server's metadata.
    pub server: Option<papo_api::wire::Server>,
    /// Channels in backend order.
    pub channels: Vec<papo_api::wire::Channel>,
    /// Role definitions.
    pub roles: Vec<papo_api::wire::Role>,
    /// The roster.
    pub members: Vec<papo_api::wire::UserSummary>,
    /// Requested full profiles.
    pub profiles: Vec<papo_api::wire::UserProfile>,
    /// Direct conversations.
    pub dms: Vec<papo_api::wire::DirectConversation>,
    /// Blocked people.
    pub blocks: Vec<papo_api::wire::UserSummary>,
}

impl ResourceSnapshot {
    /// A snapshot of fictional people and places for a client's preview (an appearance preview,
    /// a first-run showroom), admitted through the same converters as a session's data and seen
    /// as `own`. Its generation is [`Generation::FICTIONAL`], which no session has, so it can
    /// never be applied to or mistaken for an account's resources.
    /// # Errors
    /// Returns the group and fault of malformed rows.
    pub fn fictional(
        rows: SnapshotRows,
        own: crate::Uuid,
    ) -> Result<Self, crate::ResourceRejection> {
        Self::admit(Generation::FICTIONAL, rows, own)
    }

    /// Builds a deterministic projection fixture through the production admission converters.
    /// # Errors
    /// Returns the group and fault of malformed input.
    #[cfg(any(test, feature = "test-support"))]
    pub fn fixture(
        generation: Generation,
        server: Option<papo_api::wire::Server>,
        channels: Vec<papo_api::wire::Channel>,
        roles: Vec<papo_api::wire::Role>,
        members: Vec<papo_api::wire::UserSummary>,
        profiles: Vec<papo_api::wire::UserProfile>,
        dms: Vec<papo_api::wire::DirectConversation>,
        blocks: Vec<papo_api::wire::UserSummary>,
        own: crate::Uuid,
    ) -> Result<Self, crate::ResourceRejection> {
        let rows = SnapshotRows {
            server,
            channels,
            roles,
            members,
            profiles,
            dms,
            blocks,
        };
        Self::admit(generation, rows, own)
    }

    /// `rows` admitted as one mutation onto an empty image at `generation`.
    fn admit(
        generation: Generation,
        rows: SnapshotRows,
        own: crate::Uuid,
    ) -> Result<Self, crate::ResourceRejection> {
        use crate::resource_admission::{Admission, Diagnostics};
        use papo_api::wire::{
            BlockListResponse, ChannelList, DirectConversationList, RoleListResponse,
        };
        let SnapshotRows {
            server,
            channels,
            roles,
            members,
            profiles,
            dms,
            blocks,
        } = rows;
        let diagnostics = Diagnostics::default();
        let admission = Admission::for_disk(own, &diagnostics);
        let requested: Vec<_> = profiles
            .iter()
            .filter_map(|p| p.id.as_deref()?.parse().ok())
            .collect();
        let facts = AdmittedMutation {
            server: server.map(|row| admission.server(row)).transpose()?,
            channels: Some(admission.channels(ChannelList {
                channels: Some(channels),
            })?),
            roles: Some(admission.roles(RoleListResponse { roles: Some(roles) })?),
            roster: Some(admission.cached_members(members)?),
            profiles: vec![admission.profiles(&requested, Some(profiles))?],
            dms: Some(admission.dms(DirectConversationList { dms })?),
            blocks: Some(admission.blocks(BlockListResponse { users: blocks })?),
            ..Default::default()
        };
        facts.apply(&Self::empty(generation), own)
    }
}

#[cfg(test)]
impl ResourceSnapshot {
    /// Minimal admitted text channels used by isolated cache/history fixtures.
    pub(crate) fn text_channels(
        ids: impl IntoIterator<Item = crate::Uuid>,
    ) -> ResourceTable<ChannelRecord> {
        ResourceTable::from_rows(ids.into_iter().map(|id| {
            (
                id,
                ChannelRecord {
                    id,
                    kind: ChannelKind::Text,
                    name: None,
                    position: None,
                    topic: None,
                    parent_id: None,
                    permissions: Vec::new(),
                    created_at: None,
                    last_message: None,
                    last_read_message: None,
                    last_read_at: None,
                    notification_settings: None,
                },
            )
        }))
    }
}

#[cfg(feature = "test-support")]
impl ResourceSnapshot {
    /// Applies one targeted summary through production admission and mutation for projection tests.
    /// # Errors
    /// Returns the rejected group and fault for malformed or misattributed evidence.
    pub fn with_summary_for_test(
        &self,
        own: crate::Uuid,
        requested: crate::Uuid,
        summary: papo_api::wire::UserSummary,
    ) -> Result<Self, crate::ResourceRejection> {
        let diagnostics = crate::resource_admission::Diagnostics::default();
        let admission = crate::resource_admission::Admission::for_disk(own, &diagnostics);
        AdmittedMutation {
            summaries: vec![admission.summaries(&[requested], vec![summary])?],
            ..Default::default()
        }
        .apply(self, own)
    }
}

#[cfg(test)]
mod sequence_tests;

#[cfg(test)]
mod sharing_tests;

#[cfg(test)]
mod user_observation_tests;

#[cfg(test)]
mod fictional_tests {
    //! A fictional snapshot is admitted as a session's data is, at an epoch no session has.

    use super::{ResourceSnapshot, SnapshotRows};
    use crate::{Generation, Uuid};
    use papo_api::wire::UserSummary;

    fn person(id: u128, name: &str) -> UserSummary {
        UserSummary {
            id: Some(Uuid::from_u128(id).to_string()),
            username: Some(name.to_lowercase()),
            nickname: papo_api::NullableText::Value(name.to_owned()),
            ..UserSummary::default()
        }
    }

    #[test]
    fn fictional_people_are_named_as_a_sessions_are_at_an_epoch_no_session_has() {
        let rows = SnapshotRows {
            members: vec![person(1, "Lia"), person(2, "Tomás")],
            ..SnapshotRows::default()
        };
        let snapshot = ResourceSnapshot::fictional(rows, Uuid::from_u128(9)).unwrap();
        assert_eq!(snapshot.generation, Generation::FICTIONAL);
        let lia = snapshot.users.get(Uuid::from_u128(1)).unwrap();
        assert_eq!(lia.nickname.as_deref(), Some("Lia"));
        assert_eq!(
            snapshot
                .users
                .get(Uuid::from_u128(2))
                .unwrap()
                .username
                .as_deref(),
            Some("tomás")
        );
    }
}
