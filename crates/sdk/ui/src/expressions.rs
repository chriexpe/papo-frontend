//! Metadata-only expression lists and availability projection; core owns persistence/catalog and clients own pickers.

use papo_sdk::{
    ExpressionError, ExpressionFault, ExpressionSnapshot, ExpressionUpdate, Gif, MediaSource,
    Session, Sticker,
};
use std::sync::Arc;

/// Stable source identity, independent of translated labels or client widget state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExpressionId {
    /// Provider GIF identity.
    Gif(String),
    /// Server emoji/sticker UUID.
    Sticker(papo_sdk::Uuid),
}
/// One picker-ready metadata entry. Image bytes always come from the account media service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpressionItem {
    /// Stable identity, never inferred from a display name.
    pub id: ExpressionId,
    /// Provider title or last saved server sticker name.
    pub name: String,
    /// GIF encoded source; sticker sources resolve through the current emoji metadata page by ID.
    pub media: Option<MediaSource>,
}
/// Durable expression presentation without native picker, decoder or animation ownership.
#[derive(Clone, Default)]
pub struct ExpressionLibraryView {
    /// Saved GIFs and stickers in their respective durable newest-first order.
    pub favorites: Vec<ExpressionItem>,
    /// Confirmed sent GIFs, newest first; selection/Failed/Unknown do not appear.
    pub recents: Vec<ExpressionItem>,
    /// Account-scoped confirmed reaction counts, descending frequency.
    pub reaction_usage: Vec<(String, u64)>,
}
/// Coalesced view image, durable fault or revocation.
#[derive(Clone)]
pub enum ExpressionLibraryUpdate {
    /// Complete accepted metadata projection.
    Reset(Arc<ExpressionLibraryView>),
    /// Independent durable library failure; live-provider availability is reported by catalog calls.
    Fault(ExpressionFault),
    /// Account capability was revoked.
    Closed,
}
/// Account-owned observable projection of the shared expression library.
#[derive(Clone)]
pub struct ExpressionLibraryService {
    session: Session,
}
impl ExpressionLibraryService {
    /// Binds the projection to one revocable account capability.
    #[must_use]
    pub fn new(session: Session) -> Self {
        Self { session }
    }
    /// Subscribes to committed library images; clients may coalesce intermediate updates.
    /// # Errors
    /// Returns account revocation or initial durable/executor failures.
    pub async fn subscribe(&self) -> Result<ExpressionLibrarySubscription, ExpressionError> {
        let updates = self.session.expressions().subscribe().await?;
        let baseline = project(&updates.borrow());
        Ok(ExpressionLibrarySubscription { baseline, updates })
    }
}
/// Atomic baseline plus coalesced subsequent expression changes.
pub struct ExpressionLibrarySubscription {
    /// Initial accepted view or typed fault/closure.
    pub baseline: ExpressionLibraryUpdate,
    updates: tokio::sync::watch::Receiver<ExpressionUpdate>,
}
impl ExpressionLibrarySubscription {
    /// Waits for the next coalesced view. A closed account always yields Closed.
    pub async fn next(&mut self) -> ExpressionLibraryUpdate {
        if self.updates.changed().await.is_err() {
            return ExpressionLibraryUpdate::Closed;
        }
        project(&self.updates.borrow_and_update())
    }
}
fn gif(value: &Gif) -> ExpressionItem {
    ExpressionItem {
        id: ExpressionId::Gif(value.id.clone()),
        name: value.title.clone(),
        media: Some(MediaSource::External(value.url.clone())),
    }
}
fn sticker(value: &Sticker) -> ExpressionItem {
    ExpressionItem {
        id: ExpressionId::Sticker(value.id),
        name: value.name.clone(),
        media: None,
    }
}
fn view(snapshot: &ExpressionSnapshot) -> ExpressionLibraryView {
    ExpressionLibraryView {
        favorites: snapshot
            .favorite_gifs
            .iter()
            .map(gif)
            .chain(snapshot.favorite_stickers.iter().map(sticker))
            .collect(),
        recents: snapshot.recent_gifs.iter().map(gif).collect(),
        reaction_usage: snapshot.reaction_usage.clone(),
    }
}
fn project(update: &ExpressionUpdate) -> ExpressionLibraryUpdate {
    match update {
        ExpressionUpdate::Ready(snapshot) => {
            ExpressionLibraryUpdate::Reset(Arc::new(view(snapshot)))
        }
        ExpressionUpdate::Fault(fault) => ExpressionLibraryUpdate::Fault(*fault),
        _ => ExpressionLibraryUpdate::Closed,
    }
}
