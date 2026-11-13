//! Account resource metadata snapshots; persistence and reconciliation live separately.

use crate::Generation;
use papo_api::wire::{Channel, DirectConversation, Role, Server, UserProfile, UserSummary};
use std::sync::Arc;

/// One account's coherent resource image, never an authorization decision.
/// Live presence/typing belong to `ActivitySnapshot`; history and unread state have separate services.
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
        }
    }
}
