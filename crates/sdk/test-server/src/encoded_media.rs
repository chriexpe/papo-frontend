//! Scripted encoded representations for SDK fault tests; production Go handlers define authorization semantics.

use crate::{
    Error, TestServer,
    response::{Reply, problem},
    server::State,
};
use hyper::header;
use std::collections::{BTreeMap, BTreeSet};

/// One synthetic representation and its explicitly scripted access result.
#[derive(Clone)]
pub struct EncodedMedia {
    /// Exact route under attachments, media, avatars, emojis or the catalog fixture.
    pub path: String,
    /// Encoded response body.
    pub bytes: Vec<u8>,
    /// Allowed fixture user IDs; empty denotes anonymous/public content.
    pub users: BTreeSet<String>,
    /// Optional `ETag` for conditional avatar/provider tests.
    pub etag: Option<String>,
    /// Successful or injected failure status.
    pub status: u16,
    /// Optional scripted partial-representation header, including deliberately invalid values.
    pub content_range: Option<String>,
}
/// Header-only observation; never retains credential values or user search text.
#[derive(Clone)]
pub struct EncodedMediaObservation {
    /// Whether any cookie header was supplied.
    pub cookie: bool,
    /// Whether an Authorization header was supplied.
    pub authorization: bool,
    /// Whether If-None-Match was supplied.
    pub conditional: bool,
    /// Requested byte interval, without credentials or message data.
    pub range: Option<String>,
}
#[derive(Default)]
pub(crate) struct Media {
    entries: BTreeMap<String, EncodedMedia>,
    observations: Vec<EncodedMediaObservation>,
}
impl TestServer {
    /// Installs bounded synthetic bytes and a scripted grant; does not model production permissions.
    /// # Errors
    /// Returns invalid route/size/status configuration.
    pub async fn set_encoded_media(&self, value: EncodedMedia) -> Result<(), Error> {
        if ![
            "/attachments/",
            "/media/",
            "/users/",
            "/webhooks/",
            "/emojis",
            "/catalog/",
        ]
        .iter()
        .any(|prefix| value.path.starts_with(prefix))
            || value.bytes.len() > 100 * 1024 * 1024
            || !(200..=599).contains(&value.status)
        {
            return Err(Error::Configuration);
        }
        self.state
            .lock()
            .await
            .media
            .entries
            .insert(value.path.clone(), value);
        Ok(())
    }
    /// Takes header observations without retaining any secret material.
    pub async fn take_media_observations(&self) -> Vec<EncodedMediaObservation> {
        std::mem::take(&mut self.state.lock().await.media.observations)
    }
}
pub(crate) fn handle(
    state: &mut State,
    method: &str,
    path: &str,
    user: Option<&str>,
    headers: &header::HeaderMap,
) -> Option<Reply> {
    if method != "GET" {
        return None;
    }
    let value = state.media.entries.get(path)?;
    if state.media.observations.len() < 1024 {
        state.media.observations.push(EncodedMediaObservation {
            cookie: headers.contains_key(header::COOKIE),
            authorization: headers.contains_key(header::AUTHORIZATION),
            conditional: headers.contains_key(header::IF_NONE_MATCH),
            range: headers
                .get(header::RANGE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
        });
    }
    if !value.users.is_empty() && !user.is_some_and(|user| value.users.contains(user)) {
        return Some(problem(403, "forbidden", path));
    }
    if value.status >= 400 {
        return Some(problem(value.status, "internal", path));
    }
    let unchanged = value.etag.as_ref().is_some_and(|etag| {
        headers
            .get(header::IF_NONE_MATCH)
            .is_some_and(|value| value.as_bytes() == etag.as_bytes())
    });
    let mut reply = Reply::empty();
    reply.status = if unchanged { 304 } else { value.status };
    reply.encoded = Some(if unchanged {
        Vec::new()
    } else {
        value.bytes.clone()
    });
    if let Some(etag) = &value.etag {
        reply.headers.push(("etag".into(), etag.clone()));
    }
    if let Some(range) = &value.content_range {
        reply.headers.push(("content-range".into(), range.clone()));
    }
    Some(reply)
}
