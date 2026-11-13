//! Bounded backend and anonymous network fetching; account references and file installation live elsewhere.

use super::{MediaError, MediaSource, index::Reference};
use crate::{Session, generation::Epoch};

pub(super) async fn download(
    session: &Session,
    epoch: &Epoch,
    source: &MediaSource,
    previous: Option<&Reference>,
) -> Result<papo_api::media::MediaResponse, MediaError> {
    if let MediaSource::External(url) = source {
        return external(url, &epoch.cancel).await;
    }
    let rest = authenticated(session, epoch).await?;
    let work = async {
        let response = match source {
            MediaSource::Range {
                source,
                start_bytes,
                end_bytes,
            } => {
                let range = Some(papo_api::media::ByteRange::Bounded {
                    start_bytes: *start_bytes,
                    end_bytes: *end_bytes,
                });
                match source.as_ref() {
                    MediaSource::Blob(id) => rest.media_bytes(id, range).await?,
                    MediaSource::Attachment(id) => rest.attachment_bytes(*id, range).await?,
                    MediaSource::EmbedVideo(id) => rest.embed_video_bytes(*id, range).await?,
                    _ => return Err(MediaError::InvalidData),
                }
            }
            MediaSource::ServerIcon => {
                use base64::Engine;
                let server = rest.server().await.map_err(crate::ClientError::from)?;
                let raw = server.icon_blob.ok_or(MediaError::Unavailable)?;
                if raw.len() > 100 * 1024 * 1024 {
                    return Err(MediaError::TooLarge);
                }
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(raw)
                    .map_err(|_| MediaError::InvalidData)?;
                papo_api::media::MediaResponse {
                    status: 200,
                    bytes,
                    headers: reqwest::header::HeaderMap::new(),
                }
            }
            MediaSource::Blob(id) => rest.media_bytes(id, None).await?,
            MediaSource::Attachment(id) => rest.attachment_bytes(*id, None).await?,
            MediaSource::Thumbnail(id) => rest.attachment_thumbnail_bytes(*id).await?,
            MediaSource::EmbedVideo(id) => rest.embed_video_bytes(*id, None).await?,
            MediaSource::UserAvatar { user: id, .. }
            | MediaSource::WebhookAvatar { webhook: id, .. } => {
                rest.conditional_avatar_bytes(
                    *id,
                    matches!(source, MediaSource::WebhookAvatar { .. }),
                    previous.and_then(|r| r.etag.as_deref()),
                    previous.and_then(|r| r.modified.as_deref()),
                )
                .await?
            }
            MediaSource::External(_) | MediaSource::Emoji { .. } => {
                return Err(MediaError::Unavailable);
            }
        };
        Ok(response)
    };
    tokio::select! {
        biased;
        () = epoch.cancel.cancelled() => Err(crate::ClientError::Superseded.into()),
        result = tokio::time::timeout(session.client.inner.config.request_timeout, work) => result.map_err(|_| crate::ClientError::Timeout)?,
    }
}

pub(super) async fn authenticated(
    session: &Session,
    epoch: &Epoch,
) -> Result<papo_api::RestClient, MediaError> {
    let secret = session
        .client
        .inner
        .secrets
        .read(session.inner.key, &epoch.cancel)
        .await
        .map_err(crate::ClientError::from)?
        .ok_or(crate::ClientError::SignedOut)?;
    let rest = session
        .client
        .inner
        .transport(&session.inner.address, Some(secret.expose()))?;
    let work = async {
        let who = session.client.inner.request(rest.whoami()).await?;
        if who.id.as_deref().and_then(|id| id.parse().ok()) != Some(session.account().user()) {
            return Err(crate::ClientError::Identity.into());
        }
        session.inner.epochs.validate(epoch.generation)?;
        Ok(rest)
    };
    tokio::select! {biased; ()=epoch.cancel.cancelled()=>Err(crate::ClientError::Superseded.into()),result=work=>result}
}

pub(crate) fn anonymous_client() -> Result<reqwest::Client, MediaError> {
    reqwest::Client::builder()
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|_| MediaError::InvalidData)
}

pub(super) fn denied(error: &MediaError) -> bool {
    matches!(
        error,
        MediaError::Transport(papo_api::media::MediaError::Api(
            papo_api::Error::Http {
                status: 401 | 403 | 404 | 410
            } | papo_api::Error::Problem {
                status: 401 | 403 | 404 | 410,
                ..
            }
        ))
    )
}

async fn external(
    url: &str,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<papo_api::media::MediaResponse, MediaError> {
    let url = url::Url::parse(url).map_err(|_| MediaError::InvalidData)?;
    if url.scheme() != "https" || !url.username().is_empty() || url.password().is_some() {
        return Err(MediaError::InvalidData);
    }
    let work = async {
        let mut response = anonymous_client()?
            .get(url)
            .send()
            .await
            .map_err(|_| MediaError::Unavailable)?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(MediaError::Unavailable);
        }
        let headers = response.headers().clone();
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| MediaError::Unavailable)?
        {
            if chunk.len() > (100 * 1024 * 1024_usize).saturating_sub(bytes.len()) {
                return Err(MediaError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(papo_api::media::MediaResponse {
            status: 200,
            bytes,
            headers,
        })
    };
    tokio::select! { biased; ()=cancel.cancelled()=>Err(crate::ClientError::Superseded.into()), result=work=>result }
}
