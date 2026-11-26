//! Owns the fixture server's custom emoji roster and its pages; leaves authorization and the
//! SDK's catalog semantics outside.

use std::collections::BTreeMap;

use papo_api::wire::{Emoji, EmojiListResponse};
use serde_json::json;

use base64::Engine;

use crate::{
    TestServer,
    resources::Resources,
    response::{Reply, problem},
};

/// The backend's page size for `GET /emojis`.
const PAGE_LEN: usize = 25;

/// One of the fixture server's custom emoji.
#[derive(Clone, Debug)]
pub struct ServerEmoji {
    /// Backend UUID.
    pub id: String,
    /// Shortcode name without colons.
    pub name: String,
    /// Encoded image format (`png`, `gif`, …).
    pub format: String,
    /// Creation time, also the list's order.
    pub created_at: String,
    /// The encoded image; sent base64 as the backend does.
    pub image: Vec<u8>,
}

impl TestServer {
    /// Replaces the server's custom emoji. The pinned backend has no emoji event, so clients
    /// see the change only when they list again.
    pub async fn set_emojis(&self, emojis: Vec<ServerEmoji>) {
        self.state.lock().await.resources.emojis = emojis
            .into_iter()
            .map(|emoji| Emoji {
                id: Some(emoji.id),
                name: Some(emoji.name),
                format: Some(emoji.format),
                image_blob: Some(base64::engine::general_purpose::STANDARD.encode(emoji.image)),
                created_by: None,
                created_at: Some(emoji.created_at),
            })
            .collect();
    }

    /// How many `GET /emojis` pages the server has answered.
    pub async fn emoji_pages_served(&self) -> u64 {
        self.state.lock().await.resources.emoji_pages_served
    }
}

impl Resources {
    /// One page of the roster in `(created_at, id)` order after the `since` / `last_id` cursor,
    /// as the backend pages it.
    pub(crate) fn emoji_page(&mut self, query: &str) -> Reply {
        self.emoji_pages_served = self.emoji_pages_served.saturating_add(1);
        let parameters: BTreeMap<_, _> = url::form_urlencoded::parse(query.as_bytes()).collect();
        if parameters
            .get("order")
            .is_some_and(|order| order.as_ref() != "asc")
        {
            return problem(400, "invalid-param", "/emojis");
        }
        let since = parameters.get("since").map(AsRef::as_ref);
        let last_id = parameters.get("last_id").map_or("", AsRef::as_ref);
        let key = |emoji: &Emoji| {
            (
                emoji.created_at.clone().unwrap_or_default(),
                emoji.id.clone().unwrap_or_default(),
            )
        };
        let mut after: Vec<&Emoji> = self
            .emojis
            .iter()
            .filter(|emoji| {
                since.is_none_or(|since| {
                    let (created_at, id) = key(emoji);
                    (created_at.as_str(), id.as_str()) > (since, last_id)
                })
            })
            .collect();
        after.sort_by_key(|emoji| key(emoji));
        let has_more = after.len() > PAGE_LEN;
        let emojis = after.into_iter().take(PAGE_LEN).cloned().collect();
        Reply::json(
            200,
            json!(EmojiListResponse {
                emojis: Some(emojis),
                has_more: Some(has_more),
            }),
        )
    }
}
