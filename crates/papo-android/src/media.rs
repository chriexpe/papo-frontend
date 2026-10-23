use super::*;
use base64::Engine;
use papo_core::api::{
    client::Api,
    models::{Attachment, EmbedPlayback, LinkPreview},
};
use serde::Deserialize;

#[derive(Clone, Debug, uniffi::Enum)]
pub enum MediaResource {
    Attachment { id: String, thumbnail: bool },
    EmbedVideo { id: String },
    ProfileBanner { user_id: String },
}
#[derive(Clone, Debug, uniffi::Record)]
pub struct MediaRequest {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub cache_key: String,
    pub origin: String,
}
#[derive(Clone, Debug, Deserialize, serde::Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentView {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub kind: String,
    pub status: String,
    pub size_bytes: i64,
    pub thumbnail: bool,
    #[serde(default)]
    pub spoiler: bool,
}
impl From<&Attachment> for AttachmentView {
    fn from(a: &Attachment) -> Self {
        let mime = a.mime().to_ascii_lowercase();
        let kind = match a.kind() {
            papo_core::api::models::Kind::Image => "image",
            papo_core::api::models::Kind::Video => "video",
            papo_core::api::models::Kind::Audio => "audio",
            papo_core::api::models::Kind::Other => "file",
        };
        Self {
            id: a.id.clone(),
            name: a.name().to_owned(),
            mime,
            kind: kind.into(),
            status: a.moderation_status.clone().unwrap_or_default(),
            size_bytes: a.size_bytes,
            thumbnail: a.thumbnail_id.is_some(),
            spoiler: a.spoiler(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, serde::Serialize, uniffi::Record)]
pub struct EmbedFieldView {
    pub name: String,
    pub value: String,
    pub inline: bool,
}
#[derive(Clone, Debug, Deserialize, serde::Serialize, uniffi::Record)]
#[serde(rename_all = "camelCase")]
pub struct EmbedView {
    pub id: String,
    pub kind: String,
    pub source_type: String,
    pub native_media: bool,
    pub url: String,
    pub title: String,
    pub description: String,
    pub provider: String,
    pub color: String,
    pub author: String,
    pub author_url: String,
    pub has_author_image: bool,
    pub footer: String,
    pub has_image: bool,
    pub width: i32,
    pub height: i32,
    pub mime: String,
    pub iframe_url: String,
    pub fields: Vec<EmbedFieldView>,
    pub revision: String,
}
impl From<&LinkPreview> for EmbedView {
    fn from(e: &LinkPreview) -> Self {
        let has_image = e.image.is_some() || e.thumbnail.is_some() || e.image_data.is_some();
        let animated = e.animated();
        let kind = match e.playback() {
            Some(EmbedPlayback::Video) => "video",
            Some(EmbedPlayback::YouTube) => "iframe",
            _ if animated => "gif",
            _ if has_image => "image",
            _ => "link",
        };
        let (width, height) = e.media_dimensions().unwrap_or((0, 0));
        let mut fields = e.fields.iter().collect::<Vec<_>>();
        fields.sort_by_key(|f| f.position);
        Self {
            id: e.id.clone(),
            kind: kind.into(),
            source_type: e.source_type.clone().unwrap_or_else(|| "link".into()),
            native_media: e.native_media(),
            url: e.url.clone().unwrap_or_default(),
            title: e.title.clone().unwrap_or_default(),
            description: e.description.clone().unwrap_or_default(),
            provider: e
                .site_name
                .clone()
                .or(e.provider.clone())
                .unwrap_or_default(),
            color: e.color.clone().unwrap_or_default(),
            author: e
                .author
                .as_ref()
                .and_then(|a| a.name.clone())
                .unwrap_or_default(),
            author_url: e
                .author
                .as_ref()
                .and_then(|a| a.url.clone())
                .unwrap_or_default(),
            has_author_image: e.author.as_ref().is_some_and(|a| a.media.is_some())
                || e.author_image_data.is_some(),
            footer: e
                .footer
                .as_ref()
                .and_then(|f| f.text.clone())
                .unwrap_or_default(),
            has_image,
            width: width.min(i32::MAX as u32) as i32,
            height: height.min(i32::MAX as u32) as i32,
            mime: e
                .video
                .as_ref()
                .or(e.thumbnail.as_ref())
                .or(e.image.as_ref())
                .and_then(|m| m.mime_type.clone())
                .unwrap_or_default(),
            iframe_url: if kind == "iframe" {
                e.video
                    .as_ref()
                    .and_then(|v| v.url.clone())
                    .or(e.embed_url.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            },
            fields: fields
                .into_iter()
                .map(|f| EmbedFieldView {
                    name: f.name.clone(),
                    value: f.value.clone(),
                    inline: f.inline,
                })
                .collect(),
            revision: e
                .fetched_at
                .or(e.created_at)
                .map(|t| t.to_rfc3339())
                .unwrap_or_default(),
        }
    }
}

pub fn check_embed(runtime: &ServerRuntime, id: &str) -> Result<(), String> {
    if runtime.store.me.is_empty() {
        return Err("Sign in to load media".into());
    }
    if !runtime
        .store
        .media_messages()
        .any(|m| m.previews.iter().any(|e| e.id == id))
    {
        return Err("Embed is no longer in the conversation".into());
    }
    Ok(())
}

pub fn request(runtime: &ServerRuntime, resource: MediaResource) -> Result<MediaRequest, String> {
    let me = &runtime.store.me;
    if me.is_empty() {
        return Err("Sign in to load media".into());
    }
    let (id, segments, kind) = match resource {
        MediaResource::Attachment { id, thumbnail } => {
            let attachment = runtime
                .store
                .media_messages()
                .flat_map(|m| &m.attachments)
                .find(|a| a.id == id)
                .ok_or("Attachment is no longer in the conversation")?;
            if attachment.blocked() {
                return Err("This attachment is not available".into());
            }
            let segments = if thumbnail {
                vec!["attachments".into(), id.clone(), "thumbnail".into()]
            } else {
                vec!["attachments".into(), id.clone()]
            };
            (
                id,
                segments,
                if thumbnail { "thumbnail" } else { "attachment" },
            )
        }
        MediaResource::ProfileBanner { user_id } => {
            let sha = runtime
                .store
                .profiles
                .get(&user_id)
                .and_then(|p| p.banner.clone())
                .ok_or("Profile banner is unavailable")?;
            (sha.clone(), vec!["media".into(), sha], "profile-banner")
        }
        MediaResource::EmbedVideo { id } => {
            check_embed(runtime, &id)?;
            let embed = runtime
                .store
                .media_messages()
                .flat_map(|m| &m.previews)
                .find(|e| e.id == id)
                .ok_or("Embed not found")?;
            if embed.playback() != Some(EmbedPlayback::Video) {
                return Err("Embed is not a direct video".into());
            }
            (
                id.clone(),
                vec!["embeds".into(), id, "video".into()],
                "embed-video",
            )
        }
    };
    let url = resource_url(&runtime.url, &segments)?;
    let token = runtime
        .net
        .session
        .token()
        .ok_or("Your session is no longer available")?;
    let headers = HashMap::from([("Cookie".into(), format!("Auth={token}"))]);
    Ok(MediaRequest {
        origin: url.origin().ascii_serialization(),
        url: url.to_string(),
        headers,
        cache_key: format!("{}/{me}/{kind}/{id}", runtime.server_key),
    })
}

fn resource_url(base: &str, segments: &[String]) -> Result<url::Url, String> {
    let mut url = url::Url::parse(base).map_err(|e| e.to_string())?;
    url.path_segments_mut()
        .map_err(|_| "Invalid media origin")?
        .pop_if_empty()
        .extend(segments);
    Ok(url)
}

pub fn embed_image(api: Api, id: String, author: bool) -> Result<Vec<u8>, String> {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    let runtime = RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("media runtime")
    });
    let embed = runtime
        .block_on(api.link_preview(&id))
        .map_err(|e| e.to_string())?;
    let data = if author {
        embed.author_image_data
    } else {
        embed.image_data
    }
    .ok_or("The preview image is unavailable")?;
    if data.len() > 32 * 1024 * 1024 {
        return Err("The preview image is too large".into());
    }
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| e.to_string())
}

pub fn demo_request(resource: MediaResource) -> Result<MediaRequest, String> {
    let (id, file) = match resource {
        MediaResource::Attachment { id, thumbnail } => {
            let file = match (id.as_str(), thumbnail) {
                ("demo-photo", _) => "landscape.png",
                ("demo-portrait", _) => "portrait.png",
                ("demo-gif", _) => "animation.gif",
                ("demo-video", true) => "landscape.png",
                ("demo-video", false) => "clip.mp4",
                ("demo-audio", _) => "voice-note.m4a",
                ("demo-file", _) => "notes.txt",
                _ => return Err("Demo media not found".into()),
            };
            (id, file)
        }
        MediaResource::EmbedVideo { id } if id == "embed-video" => (id, "clip.mp4"),
        MediaResource::ProfileBanner { user_id } if user_id == "ana" => (user_id, "landscape.png"),
        _ => return Err("Demo media not found".into()),
    };
    Ok(MediaRequest {
        url: format!("file:///android_asset/demo/{file}"),
        headers: HashMap::new(),
        origin: "demo".into(),
        cache_key: format!("demo/{id}/{file}"),
    })
}
pub fn demo_image(id: &str) -> Vec<u8> {
    match id {
        "embed-gif" => {
            include_bytes!("../../../android/compose/src/main/assets/demo/animation.gif").to_vec()
        }
        "embed-portrait" => {
            include_bytes!("../../../android/compose/src/main/assets/demo/portrait.png").to_vec()
        }
        _ => include_bytes!("../../../android/compose/src/main/assets/demo/landscape.png").to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spoiler_filenames_survive_projection_independently_of_moderation() {
        for (name, expected) in [("SPOILER_photo.png", true), ("photo.png", false)] {
            let attachment: Attachment = serde_json::from_value(serde_json::json!({
                "id":"private", "original_file_name":name, "mime_type":"image/png", "moderation_status":"clean"
            })).unwrap();
            let view = AttachmentView::from(&attachment);
            assert_eq!(view.spoiler, expected);
            assert_eq!(view.status, "clean");
            assert_eq!(view.name, name);
        }
    }

    #[test]
    fn direct_media_keeps_custom_cards_and_webpage_previews_distinct() {
        let mut embed: LinkPreview = serde_json::from_value(serde_json::json!({
            "id":"direct", "url":"https://cdn.example/photo.png?sig=abc", "title":"Image title",
            "image":{"mime_type":"image/png","width":600,"height":400}
        }))
        .unwrap();
        assert!(EmbedView::from(&embed).native_media);
        embed.source_type = Some("custom".into());
        assert!(!EmbedView::from(&embed).native_media);
        embed.source_type = Some("link".into());
        embed.url = Some("https://example/article".into());
        assert!(!EmbedView::from(&embed).native_media);
        embed.image.as_mut().unwrap().url = embed.url.clone();
        assert!(
            EmbedView::from(&embed).native_media,
            "Opaque media URLs are confirmed by matching media metadata"
        );
        embed.video = Some(
            serde_json::from_value(
                serde_json::json!({"url":"https://www.youtube.com/embed/abc123"}),
            )
            .unwrap(),
        );
        assert!(!EmbedView::from(&embed).native_media);
    }

    #[test]
    fn static_thumbnail_does_not_hide_an_animated_original() {
        let embed: LinkPreview = serde_json::from_value(serde_json::json!({
            "id":"gif", "url":"https://cdn.example/opaque", "thumbnail":{"mime_type":"image/png"},
            "image":{"mime_type":"IMAGE/GIF; charset=binary"}
        }))
        .unwrap();
        let view = EmbedView::from(&embed);
        assert_eq!(view.kind, "gif");
        assert!(view.native_media);
    }

    #[test]
    fn backend_labeled_m4a_projects_as_audio_without_rewriting_metadata() {
        let attachment: Attachment = serde_json::from_value(serde_json::json!({
            "id":"voice", "original_file_name":"Voice note.m4a", "mime_type":"video/mp4", "size_bytes":1024
        })).unwrap();
        let view = AttachmentView::from(&attachment);
        assert_eq!(view.kind, "audio");
        assert_eq!(view.mime, "video/mp4");
    }

    #[test]
    fn media_routes_keep_server_prefix_and_encode_ids_as_one_segment() {
        for base in ["https://chat.example/papo", "https://chat.example/papo/"] {
            let route = resource_url(
                base,
                &["attachments".into(), "id/with?query#fragment".into()],
            )
            .unwrap();
            assert_eq!(
                route.as_str(),
                "https://chat.example/papo/attachments/id%2Fwith%3Fquery%23fragment"
            );
            assert_eq!(route.origin().ascii_serialization(), "https://chat.example");
        }
        assert_eq!(
            resource_url(
                "https://chat.example",
                &["embeds".into(), "id".into(), "video".into()]
            )
            .unwrap()
            .as_str(),
            "https://chat.example/embeds/id/video"
        );
    }
}
