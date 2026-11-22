//! Account-scoped observable channel metadata; SDK owns resources, access and session lifecycle.
//!
//! Backend order is preserved exactly, including tied, absent or negative positions.
//! No channels are invented, deduplicated or permission-filtered here. Permissions,
//! capabilities, navigation and presentation belong to other layers. Unread state is an overlay
//! from the SDK's durable read-point/coverage projection, never inferred from metadata timestamps.

use std::{pin::Pin, sync::Arc};

use eyeball_im::VectorDiff;
use futures_util::Stream;
use imbl::Vector;
use papo_sdk::{ChannelKind, Generation, Session, Uuid};

use crate::channel_projection;

/// Backend channel metadata with SDK-provided unread state, without inferred permissions.
///
/// Each admitted channel has a stable identity and a known kind.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChannelListItem {
    /// Backend channel identity.
    pub id: Uuid,
    /// Backend channel name, untranslated and unmodified.
    pub name: Option<String>,
    /// Admitted backend channel kind.
    pub kind: ChannelKind,
    /// Backend position metadata; does not override the backend's list order.
    pub position: Option<i64>,
    /// Backend topic metadata.
    pub topic: Option<String>,
    /// Parent category channel UUID, if nested under a category.
    pub parent_id: Option<Uuid>,
    /// Backend notification mode override stored on this channel record.
    pub notification_settings: Option<papo_sdk::BackendMode>,
    /// Exact backend timestamp string, without reparsing or precision loss.
    pub created_at: Option<String>,
    /// SDK read-point/coverage projection. Absent until this generation has verified read state.
    pub unread: Option<papo_sdk::ReadState>,
    /// Core-owned eligible mention count and unread evidence; absent before a matching image.
    pub badges: Option<Arc<papo_sdk::ChannelBadges>>,
}

/// One visible change to channel metadata, read state or badges.
///
/// Apply `diffs` in order as one batch, then advance both version and generation.
/// Unchanged projections stay silent. Coalesced source images reconcile against the last delivered rows.
/// A generation change starts a fresh baseline with one `Reset`.
#[derive(Clone, Debug)]
pub struct ChannelListUpdate {
    /// Channel-list projection version represented after applying the entire batch.
    pub version: u64,
    /// Connection epoch stamped on the committed resource snapshot.
    pub generation: Generation,
    /// Sequential channel edits; unchanged uniquely identified items retain their `Arc`.
    pub diffs: Vec<VectorDiff<Arc<ChannelListItem>>>,
}

/// Pull-driven channel edits; ends after the SDK resource watch closes.
///
/// The source watch has capacity one. Slow subscribers retain their last projection
/// and the newest source snapshot. The next batch reconciles those two images.
/// There is no background task, unbounded queue or additional Tokio runtime.
pub type ChannelListStream = Pin<Box<dyn Stream<Item = ChannelListUpdate> + Send>>;

/// Atomic channel baseline and changes strictly after that captured publication.
pub struct ChannelListSubscription {
    /// Channel-list projection version represented by `snapshot`.
    pub version: u64,
    /// Epoch belonging to that same captured version, not a separate session read.
    pub generation: Generation,
    /// Channel metadata in backend order, with generation-matching SDK unread state.
    pub snapshot: Vector<Arc<ChannelListItem>>,
    /// Subsequent batches; final unseen changes drain before closure ends the stream.
    pub stream: ChannelListStream,
}

/// Projects backend-returned channels for exactly one SDK account session.
#[derive(Clone)]
pub struct ChannelListService {
    /// The existing account capability, never a new transport or resource cache.
    session: Session,
}

impl ChannelListService {
    /// Binds to this session's account and existing SDK lifecycle.
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self { session }
    }

    /// Captures version, generation and channels atomically with the source watch.
    ///
    /// No event can fall between the baseline and stream. Reconnect generations
    /// reset the list; stopping/replacing the account closes the SDK watch and ends
    /// existing subscriptions after any final unseen publication has been drained.
    #[must_use]
    pub fn subscribe(&self) -> ChannelListSubscription {
        crate::channel_reading::subscribe(
            channel_projection::subscribe(self.session.subscribe_resources()),
            self.session.subscribe_reading(),
            self.session.subscribe_badges(),
        )
    }
}
