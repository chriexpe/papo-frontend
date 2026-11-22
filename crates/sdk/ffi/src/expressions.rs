//! Durable expression favorites/recents, custom stickers, reaction frequency, and GIPHY catalog.
//! All selection/recents policy remains inside `papo-sdk` and `papo-sdk-ui`.

use std::sync::Arc;

use futures_util::stream;
use papo_sdk::{ExpressionFault, Gif, Sticker};
use papo_sdk_ui::{
    ExpressionId, ExpressionItem, ExpressionLibraryService, ExpressionLibraryUpdate,
    ExpressionLibraryView,
};

use crate::{
    client::FfiClient,
    errors::FfiClientError,
    ids::{FfiAccountId, parse_uuid},
    media_records::FfiMediaSource,
    operation_errors::FfiExpressionError,
    subscription::{FfiSubscriptionHandle, spawn_stream_subscription},
};

/// Saved or catalog GIF metadata. Never uses display title as identifier.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiGif {
    /// Provider opaque identifier.
    pub id: String,
    /// Original HTTPS media URL.
    pub url: String,
    /// Provider attribution page URL.
    pub page_url: String,
    /// Human-readable title metadata.
    pub title: String,
}

impl From<&Gif> for FfiGif {
    fn from(v: &Gif) -> Self {
        Self {
            id: v.id.clone(),
            url: v.url.clone(),
            page_url: v.page_url.clone(),
            title: v.title.clone(),
        }
    }
}

impl From<FfiGif> for Gif {
    fn from(v: FfiGif) -> Self {
        Self {
            id: v.id,
            url: v.url,
            page_url: v.page_url,
            title: v.title,
        }
    }
}

/// Custom server sticker metadata without cached image bytes.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiSticker {
    /// Server emoji UUID string.
    pub id: String,
    /// Last confirmed or saved sticker name.
    pub name: String,
}

/// Picker expression item identifier (GIF vs custom server sticker).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiExpressionId {
    /// Provider GIF opaque ID.
    Gif {
        /// Provider GIF ID string.
        id: String,
    },
    /// Server sticker emoji UUID.
    Sticker {
        /// Emoji UUID string.
        emoji_id: String,
    },
}

/// Unified picker item carrying optional media source and GIF attribution `page_url`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiExpressionItem {
    /// Stable source identifier.
    pub id: FfiExpressionId,
    /// Provider title or sticker name.
    pub name: String,
    /// Media source for image bytes if resolvable directly.
    pub media: Option<FfiMediaSource>,
    /// Provider attribution page URL for GIFs; `None` for stickers.
    pub page_url: Option<String>,
}

impl From<&ExpressionItem> for FfiExpressionItem {
    fn from(v: &ExpressionItem) -> Self {
        Self {
            id: match &v.id {
                ExpressionId::Gif(id) => FfiExpressionId::Gif { id: id.clone() },
                ExpressionId::Sticker(id) => FfiExpressionId::Sticker {
                    emoji_id: id.to_string(),
                },
            },
            name: v.name.clone(),
            media: v.media.as_ref().map(FfiMediaSource::from_sdk),
            page_url: v.page_url.clone(),
        }
    }
}

/// Reaction frequency item ordered descending in library views.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiReactionUsageItem {
    /// Custom emoji UUID or unicode sequence key.
    pub key: String,
    /// Confirmed usage count.
    pub count: u64,
}

/// Picker library snapshot of favorites, recents, and top reaction usage.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiExpressionLibraryView {
    /// Saved favorite GIFs and stickers.
    pub favorites: Vec<FfiExpressionItem>,
    /// Recently confirmed sent GIFs.
    pub recents: Vec<FfiExpressionItem>,
    /// Frequently used reaction keys descending.
    pub reaction_usage: Vec<FfiReactionUsageItem>,
}

impl From<&ExpressionLibraryView> for FfiExpressionLibraryView {
    fn from(v: &ExpressionLibraryView) -> Self {
        Self {
            favorites: v.favorites.iter().map(Into::into).collect(),
            recents: v.recents.iter().map(Into::into).collect(),
            reaction_usage: v
                .reaction_usage
                .iter()
                .map(|(key, count)| FfiReactionUsageItem {
                    key: key.clone(),
                    count: *count,
                })
                .collect(),
        }
    }
}

/// Durable expression fault reported independently of live GIF search.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiExpressionFault {
    /// Storage read/write failed.
    Storage,
    /// Expression library payload invalid.
    InvalidData,
}

/// Expression library subscription event.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiExpressionLibraryEvent {
    /// Complete updated library projection.
    Reset {
        /// Current library contents.
        view: FfiExpressionLibraryView,
    },
    /// Independent durable fault.
    Fault {
        /// Fault category.
        fault: FfiExpressionFault,
    },
    /// Session revoked.
    Closed,
}

impl From<&ExpressionLibraryUpdate> for FfiExpressionLibraryEvent {
    fn from(upd: &ExpressionLibraryUpdate) -> Self {
        match upd {
            ExpressionLibraryUpdate::Reset(view) => Self::Reset {
                view: view.as_ref().into(),
            },
            ExpressionLibraryUpdate::Fault(fault) => Self::Fault {
                fault: match fault {
                    ExpressionFault::InvalidData => FfiExpressionFault::InvalidData,
                    // Storage and future fault variants surface as Storage.
                    _ => FfiExpressionFault::Storage,
                },
            },
            ExpressionLibraryUpdate::Closed => Self::Closed,
        }
    }
}

/// Listener for expression library updates.
#[uniffi::export(with_foreign)]
pub trait FfiExpressionLibraryListener: Send + Sync {
    /// Atomic baseline library state.
    fn on_snapshot(&self, event: FfiExpressionLibraryEvent);
    /// Subsequent coalesced library updates.
    fn on_update(&self, event: FfiExpressionLibraryEvent);
    /// Terminal notification on closure.
    fn on_closed(&self);
}

/// One GIPHY trending or search result page tagged with request generation.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiCatalogPage {
    /// Session connection epoch.
    pub generation: u64,
    /// Provider GIFs in provider order.
    pub gifs: Vec<FfiGif>,
    /// Next provider pagination offset when more pages exist.
    pub next_offset: Option<u32>,
}

#[uniffi::export]
impl FfiClient {
    /// Subscribes to durable favorites, confirmed recents, and reaction usage for `account`.
    ///
    /// # Errors
    /// Returns [`FfiExpressionError`].
    pub async fn subscribe_expressions(
        &self,
        account: FfiAccountId,
        listener: Arc<dyn FfiExpressionLibraryListener>,
    ) -> Result<Arc<FfiSubscriptionHandle>, FfiExpressionError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiExpressionError::Session { error: err })?;
        let runtime = self.runtime.clone();
        let tasks = self.tasks.clone();
        let shutdown = self.shutdown.clone();
        let callbacks = self.callbacks.clone();
        self.run_scoped(
            || FfiExpressionError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                let sub = ExpressionLibraryService::new(session).subscribe().await?;
                let baseline = FfiExpressionLibraryEvent::from(&sub.baseline);
                let l_snap = Arc::clone(&listener);
                let l_upd = Arc::clone(&listener);
                let l_end = listener;
                let stream = Box::pin(stream::unfold(sub, |mut sub| async move {
                    let next = sub.next().await;
                    if matches!(next, ExpressionLibraryUpdate::Closed) {
                        None
                    } else {
                        Some((next, sub))
                    }
                }));
                // Emit Closed via `on_end` when `next()` transitions to Closed or subscription ends.
                spawn_stream_subscription(
                    &runtime,
                    &tasks,
                    &shutdown,
                    callbacks,
                    baseline,
                    stream,
                    move |ev| {
                        l_snap.on_snapshot(ev);
                        true
                    },
                    move |ev| {
                        l_upd.on_update(FfiExpressionLibraryEvent::from(&ev));
                        true
                    },
                    move || l_end.on_closed(),
                )
                .map_err(|err| FfiExpressionError::Session { error: err })
            },
        )
        .await
    }

    /// Saves or removes a favorite GIF with FULL commit.
    ///
    /// # Errors
    /// Returns [`FfiExpressionError`].
    pub async fn favorite_gif(
        &self,
        account: FfiAccountId,
        gif: FfiGif,
        favorite: bool,
    ) -> Result<(), FfiExpressionError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiExpressionError::Session { error: err })?;
        self.run_scoped(
            || FfiExpressionError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                session
                    .expressions()
                    .favorite_gif(gif.into(), favorite)
                    .await
                    .map_err(Into::into)
            },
        )
        .await
    }

    /// Saves or removes a favorite server sticker with FULL commit.
    ///
    /// # Errors
    /// Returns [`FfiExpressionError`].
    pub async fn favorite_sticker(
        &self,
        account: FfiAccountId,
        sticker: FfiSticker,
        favorite: bool,
    ) -> Result<(), FfiExpressionError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiExpressionError::Session { error: err })?;
        let id = parse_uuid(&sticker.id).map_err(|_| FfiExpressionError::InvalidData)?;
        self.run_scoped(
            || FfiExpressionError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                session
                    .expressions()
                    .favorite_sticker(
                        Sticker {
                            id,
                            name: sticker.name,
                        },
                        favorite,
                    )
                    .await
                    .map_err(Into::into)
            },
        )
        .await
    }

    /// Fetches trending GIFs from optional external GIPHY provider without Papo credentials.
    ///
    /// # Errors
    /// Returns [`FfiExpressionError::Unavailable`] if GIPHY key wasn't configured.
    pub async fn trending_gifs(
        &self,
        account: FfiAccountId,
        offset: u32,
    ) -> Result<FfiCatalogPage, FfiExpressionError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiExpressionError::Session { error: err })?;
        self.run_scoped(
            || FfiExpressionError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                let page = session.expressions().trending(offset).await?;
                Ok(FfiCatalogPage {
                    generation: page.generation.get(),
                    gifs: page.value.gifs.iter().map(Into::into).collect(),
                    next_offset: page.value.next_offset,
                })
            },
        )
        .await
    }

    /// Searches external GIPHY catalog without altering user spelling or reordering provider results.
    ///
    /// # Errors
    /// Returns [`FfiExpressionError`].
    pub async fn search_gifs(
        &self,
        account: FfiAccountId,
        query: String,
        offset: u32,
    ) -> Result<FfiCatalogPage, FfiExpressionError> {
        let session = self
            .session_by_id(&account)
            .map_err(|err| FfiExpressionError::Session { error: err })?;
        self.run_scoped(
            || FfiExpressionError::Session {
                error: FfiClientError::Closed,
            },
            async move {
                let page = session.expressions().search(query, offset).await?;
                Ok(FfiCatalogPage {
                    generation: page.generation.get(),
                    gifs: page.value.gifs.iter().map(Into::into).collect(),
                    next_offset: page.value.next_offset,
                })
            },
        )
        .await
    }
}
