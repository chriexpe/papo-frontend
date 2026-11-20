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
            gaps: EvidenceGaps::default(),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ResourceSnapshot {
    /// Builds a deterministic projection fixture through the production admission converters.
    /// # Errors
    /// Returns the group and fault of malformed input.
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
        use crate::resource_admission::{Admission, Diagnostics};
        use papo_api::wire::{
            BlockListResponse, ChannelList, DirectConversationList, RoleListResponse,
        };
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
