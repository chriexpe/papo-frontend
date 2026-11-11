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
use papo_sdk::{Generation, Session};

use crate::channel_projection;

/// Backend channel metadata with SDK-provided unread state, without inferred permissions.
///
/// Optional fields remain optional. A missing, empty or duplicated ID cannot provide
/// stable identity, so transitions involving those lists use a full `Reset`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChannelListItem {
    /// Backend identity, when supplied; never synthesized from position or name.
    pub id: Option<String>,
    /// Backend channel name, untranslated and unmodified.
    pub name: Option<String>,
    /// Backend channel type, retained verbatim rather than assuming known variants.
    pub kind: Option<String>,
    /// Backend position metadata; does not override the backend's list order.
    pub position: Option<i64>,
    /// Backend topic metadata.
    pub topic: Option<String>,
    /// Exact backend timestamp string, without reparsing or precision loss.
    pub created_at: Option<String>,
    /// SDK read-point/coverage projection. Absent until this generation has verified read state.
    pub unread: Option<papo_sdk::ReadState>,
}

/// One complete resource publication projected onto channel metadata.
///
/// Apply `diffs` in order as one batch, then advance both version and generation.
/// Commits affecting other resources can produce empty batches. Any version gap or
/// generation change produces exactly one `Reset`, even if metadata is unchanged.
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
/// and only the newest source snapshot, coalescing skipped versions into a `Reset`.
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
        )
    }
}
