use crate::{
    bindings::{Backend, CoreError, NativeClient},
    *,
};
use papo_core::preview::{PreviewCoordinator, PreviewState};

#[derive(Clone, Debug, Default, uniffi::Record)]
pub struct EmbedDetailsView {
    pub pending: bool,
    pub author: String,
    pub author_url: String,
    pub handle: String,
    pub avatar: String,
    pub caption: Option<String>,
    pub provider: String,
    pub likes: Option<u64>,
    pub comments: Option<u64>,
    pub shares: Option<u64>,
    pub views: Option<u64>,
    pub quote: Option<EmbedQuoteView>,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct EmbedQuoteView {
    pub author: String,
    pub handle: String,
    pub caption: String,
}

#[uniffi::export]
impl NativeClient {
    /// Enrich presentation only. Backend media/relay capabilities remain authoritative.
    pub fn embed_details(
        &self,
        id: String,
        source_url: String,
    ) -> Result<EmbedDetailsView, CoreError> {
        let backend = self
            .backend
            .lock()
            .map_err(|_| CoreError::from("Native client lock failed".to_owned()))?;
        let Some(Backend::Live(client)) = backend.as_ref() else {
            return Ok(EmbedDetailsView::default());
        };
        let known = client.runtime.store.media_messages().any(|message| {
            message.previews.iter().any(|embed| {
                embed.id == id
                    && embed.source_type.as_deref() != Some("custom")
                    && (embed.url.as_deref() == Some(&source_url)
                        || papo_core::preview::extract_https_urls(&message.content)
                            .contains(&source_url))
            })
        });
        if !known {
            return Err("The embed is no longer available in this account"
                .to_owned()
                .into());
        }
        let coordinator = self.previews.get_or_init(|| {
            PreviewCoordinator::new(Arc::clone(&client.runtime.cache), Arc::new(|| {}))
        });
        let state = coordinator.get_or_request(&source_url);
        drop(backend);
        Ok(match state {
            Some(PreviewState::Loading) => EmbedDetailsView {
                pending: true,
                ..Default::default()
            },
            Some(PreviewState::Ready(preview)) => {
                let post = *preview.post;
                EmbedDetailsView {
                    pending: false,
                    author: post.author_name.unwrap_or_default(),
                    author_url: post.author_url.unwrap_or_default(),
                    handle: post.author_handle.unwrap_or_default(),
                    avatar: post.author_avatar.unwrap_or_default(),
                    caption: post.caption,
                    provider: preview.provider_name.unwrap_or_default(),
                    likes: post.likes,
                    comments: post.comments,
                    shares: post.shares,
                    views: post.views,
                    quote: post.quote.map(|quote| EmbedQuoteView {
                        author: quote.author,
                        handle: quote.handle,
                        caption: quote.caption,
                    }),
                }
            }
            _ => EmbedDetailsView::default(),
        })
    }
}
