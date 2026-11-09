//! Account resource snapshots and in-memory presence; persistence and reconciliation live separately.

use crate::Generation;
use papo_api::{
    gateway::PresenceMember,
    wire::{Channel, DirectConversation, Role, Server, UserProfile, UserSummary},
};
use std::sync::Arc;

/// One account's coherent resource image, never an authorization decision.
/// Presence is memory-only. Message history, unread counts and capability evaluation are not included.
#[derive(Clone)]
pub struct ResourceSnapshot {
    /// Monotonically increasing publication within this session, including reconcile replacements.
    pub version: u64,
    /// Connection that requested the data. Older generations must not be applied.
    pub generation: Generation,
    /// Backend server metadata, absent before the first verified cache load or reconcile.
    pub server: Option<Arc<Server>>,
    /// All channels returned by the backend, in backend order; no client-side filtering.
    pub channels: Arc<Vec<Channel>>,
    /// Backend role definitions.
    pub roles: Arc<Vec<Role>>,
    /// Lightweight membership list; full profiles are fetched in batches on demand.
    pub members: Arc<Vec<UserSummary>>,
    /// Profiles requested in this generation. Never eagerly fetched for all members.
    pub profiles: Arc<Vec<UserProfile>>,
    /// Backend direct conversations.
    pub dms: Arc<Vec<DirectConversation>>,
    /// Backend-authoritative block list.
    pub blocks: Arc<Vec<UserSummary>>,
    /// Current generation's presence, replaced by `PresenceSync`; never persisted.
    pub presence: Arc<Vec<PresenceMember>>,
}
impl ResourceSnapshot {
    pub(crate) fn empty(generation: Generation) -> Self {
        Self {
            version: 0,
            generation,
            server: None,
            channels: Arc::default(),
            roles: Arc::default(),
            members: Arc::default(),
            profiles: Arc::default(),
            dms: Arc::default(),
            blocks: Arc::default(),
            presence: Arc::default(),
        }
    }
}
