//! Authoring validation follows the backend's Unicode limits and media contract.
//! Received protected media must never be reconstructed as public input URLs.
use crate::api::models::*;

pub const MAX_EMBEDS: usize = 10;
pub const TOTAL_TEXT: usize = 6000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub index: usize,
    pub field: String,
    pub code: &'static str,
    pub limit: usize,
}

pub fn validate(inputs: &[CustomEmbedInput], attachments: &[String]) -> Vec<Issue> {
    let mut issues = vec![];
    let mut report = |index, field: &str, code, limit| {
        issues.push(Issue {
            index,
            field: field.into(),
            code,
            limit,
        });
    };
    if inputs.len() > MAX_EMBEDS {
        report(0, "embeds", "limit", MAX_EMBEDS);
    }
    let mut total = 0;
    for (index, embed) in inputs.iter().enumerate() {
        if !embed.has_content() {
            report(index, "embeds", "empty", 0);
        }
        let author = embed.author.as_ref();
        let footer = embed.footer.as_ref();
        for (field, value, max, summed) in [
            ("title", embed.title.as_str(), 256, true),
            ("description", embed.description.as_str(), 4192, true),
            ("site_name", embed.site_name.as_str(), 256, false),
            ("author", author.map_or("", |a| a.name.as_str()), 256, true),
            ("footer", footer.map_or("", |f| f.text.as_str()), 2048, true),
        ] {
            let len = value.chars().count();
            if len > max {
                report(index, field, "limit", max);
            }
            if summed {
                total += len;
            }
        }
        if !embed.color.is_empty()
            && !(embed.color.len() == 7
                && embed.color.starts_with('#')
                && embed.color[1..].bytes().all(|b| b.is_ascii_hexdigit()))
        {
            report(index, "color", "color", 0);
        }
        for (field, value, https) in [
            ("url", embed.url.as_str(), false),
            ("author_url", author.map_or("", |a| a.url.as_str()), false),
            (
                "author_icon",
                author.map_or("", |a| a.icon_url.as_str()),
                true,
            ),
        ] {
            if !value.is_empty() && !valid_url(value, https) {
                report(index, field, "url", 2048);
            }
        }
        for (field, media) in [
            ("thumbnail", &embed.thumbnail),
            ("image", &embed.image),
            ("video", &embed.video),
        ] {
            let Some(media) = media else {
                continue;
            };
            if !media.attachment_name.is_empty() {
                if field == "video"
                    || media.attachment_name.starts_with("SPOILER_")
                    || media.attachment_name.len() > 255
                    || media.attachment_name.trim() != media.attachment_name
                    || media
                        .attachment_name
                        .chars()
                        .any(|c| c.is_control() || matches!(c, '/' | '\\'))
                    || !media.url.is_empty()
                    || attachments
                        .iter()
                        .filter(|name| **name == media.attachment_name)
                        .count()
                        != 1
                {
                    report(index, field, "attachment", 0);
                }
            } else if !valid_url(&media.url, true) {
                report(index, field, "url", 2048);
            }
            let mime = if field == "video" || !media.attachment_name.is_empty() {
                media
                    .mime_type
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase()
            } else {
                media.mime_type.to_ascii_lowercase()
            };
            if (field == "video"
                && !matches!(mime.as_str(), "video/mp4" | "video/webm" | "video/ogg"))
                || (field != "video"
                    && !mime.is_empty()
                    && !matches!(
                        mime.as_str(),
                        "image/png" | "image/jpeg" | "image/webp" | "image/gif"
                    ))
            {
                report(index, field, "mime", 0);
            }
        }
        if embed.fields.len() > 25 {
            report(index, "fields", "limit", 25);
        }
        for field in &embed.fields {
            let (name, value) = (field.name.chars().count(), field.value.chars().count());
            total += name + value;
            if name > 256 {
                report(index, "field_name", "limit", 256);
            }
            if value > 1024 {
                report(index, "field_value", "limit", 1024);
            }
        }
    }
    if total > TOTAL_TEXT {
        report(0, "total", "limit", TOTAL_TEXT);
    }
    issues
}

pub fn valid_url(raw: &str, https: bool) -> bool {
    if raw.chars().count() > 2048 {
        return false;
    }
    let Ok(url) = url::Url::parse(raw.trim()) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https")
        && (!https || url.scheme() == "https")
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
}

/// A replacement list is all-or-nothing. Preserving only the editable subset
/// would delete protected embeds when the backend replaces the complete list.
pub fn editable(previews: &[LinkPreview]) -> Option<Vec<CustomEmbedInput>> {
    previews
        .iter()
        .filter(|p| p.source_type.as_deref() == Some("custom"))
        .map(|preview| {
            let mut metadata = preview.clone();
            metadata.fields.sort_by_key(|f| f.position);
            // Direct video retains its canonical public URL. Image/author blobs
            // have protected identities that the edit input cannot represent.
            let video = metadata.video.take();
            let mut input = CustomEmbedInput::from_media_free_preview(&metadata)?;
            if let Some(video) = video {
                let url = video.url?;
                let mime_type = video.mime_type?;
                if !valid_url(&url, true)
                    || !matches!(mime_type.as_str(), "video/mp4" | "video/webm" | "video/ogg")
                {
                    return None;
                }
                input.video = Some(CustomEmbedMediaInput {
                    url,
                    mime_type,
                    ..Default::default()
                });
            }
            Some(input)
        })
        .collect()
}

pub fn preview(input: &CustomEmbedInput, id: &str) -> LinkPreview {
    let image = |m: &CustomEmbedMediaInput| EmbedMedia {
        url: (!m.url.is_empty()).then(|| m.url.clone()),
        mime_type: Some(m.mime_type.clone()),
        width: None,
        height: None,
        size_bytes: None,
    };
    LinkPreview {
        id: id.into(),
        source_type: Some("custom".into()),
        fetch_method: Some("manual".into()),
        provider: None,
        site_name: Some(input.site_name.clone()),
        url: Some(input.url.clone()),
        title: Some(input.title.clone()),
        description: Some(input.description.clone()),
        color: Some(input.color.clone()),
        author: input.author.as_ref().map(|a| EmbedAuthor {
            name: Some(a.name.clone()),
            url: Some(a.url.clone()),
            media: (!a.icon_url.is_empty()).then(|| EmbedMedia {
                url: Some(a.icon_url.clone()),
                mime_type: None,
                width: None,
                height: None,
                size_bytes: None,
            }),
        }),
        footer: input.footer.as_ref().map(|f| EmbedFooter {
            text: Some(f.text.clone()),
            icon: None,
        }),
        thumbnail: input.thumbnail.as_ref().map(image),
        image: input.image.as_ref().map(image),
        video: input.video.as_ref().map(image),
        fields: input
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| EmbedField {
                position: i as i32,
                name: f.name.clone(),
                value: f.value.clone(),
                inline: f.inline,
            })
            .collect(),
        embed_url: None,
        created_at: None,
        fetched_at: None,
        image_data: None,
        author_image_data: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_limits_aggregate_and_attachment_ambiguity_match_backend() {
        let mut embed = CustomEmbedInput {
            title: "😀".repeat(256),
            ..Default::default()
        };
        assert!(validate(&[embed.clone()], &[]).is_empty());
        embed.title.push('a');
        assert!(!validate(&[embed.clone()], &[]).is_empty());
        embed.title.clear();
        embed.description = "x".repeat(4000);
        assert!(
            validate(&[embed.clone(), embed], &[])
                .iter()
                .any(|i| i.field == "total")
        );
        let embed = CustomEmbedInput {
            image: Some(CustomEmbedMediaInput {
                attachment_name: "photo.png".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(validate(std::slice::from_ref(&embed), &["photo.png".into()]).is_empty());
        assert!(!validate(&[embed], &["photo.png".into(), "photo.png".into()]).is_empty());
    }
    #[test]
    fn direct_video_can_be_edited_but_spoiler_files_stay_independent() {
        let input = CustomEmbedInput {
            video: Some(CustomEmbedMediaInput {
                url: "https://example.org/clip.mp4".into(),
                mime_type: "video/mp4".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(editable(&[preview(&input, "video")]), Some(vec![input]));
        let input = CustomEmbedInput {
            image: Some(CustomEmbedMediaInput {
                attachment_name: "SPOILER_photo.png".into(),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(!validate(&[input], &["SPOILER_photo.png".into()]).is_empty());
    }
    #[test]
    fn private_media_is_not_reconstructed_and_public_schemes_are_checked() {
        let mut embed = preview(
            &CustomEmbedInput {
                title: "Kept".into(),
                ..Default::default()
            },
            "original",
        );
        assert!(editable(&[embed.clone()]).is_some());
        embed.image = Some(EmbedMedia {
            url: None,
            mime_type: Some("image/png".into()),
            width: Some(20),
            height: Some(20),
            size_bytes: None,
        });
        assert!(editable(&[embed]).is_none());
        assert!(!valid_url("http://example.com/p.png", true));
        assert!(!valid_url("https://user:password@example.com/p.png", true));
        assert!(!valid_url("https://example.com:444/p.png", true));
        assert!(valid_url("https://example.com/p.png", true));
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    use crate::api::client::{Api, Session, Upload};
    use std::sync::Arc;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    #[tokio::test]
    async fn multipart_embeds_and_edit_omission_are_real_backend_payloads() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let embed = CustomEmbedInput {
            title: "Native 😀".into(),
            image: Some(CustomEmbedMediaInput {
                attachment_name: "photo.png".into(),
                mime_type: "image/png".into(),
                ..Default::default()
            }),
            fields: vec![CustomEmbedFieldInput {
                name: "name".into(),
                value: "value".into(),
                inline: true,
            }],
            ..Default::default()
        };
        let expected = serde_json::to_string(&vec![embed.clone()]).unwrap();
        let server = tokio::spawn(async move {
            for index in 0..4 {
                let (stream, _) = listener.accept().await.unwrap();
                let mut reader = BufReader::new(stream);
                let mut line = String::new();
                reader.read_line(&mut line).await.unwrap();
                assert!(line.starts_with(if index == 0 {
                    "POST /messages "
                } else {
                    "PUT /messages/m "
                }));
                let mut length = 0;
                loop {
                    line.clear();
                    reader.read_line(&mut line).await.unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some((key, value)) = line.split_once(':')
                        && key.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).await.unwrap();
                let text = String::from_utf8(bytes).unwrap();
                if index == 0 {
                    assert!(text.contains("filename=\"photo.png\""));
                    assert!(text.contains("name=\"embeds\""));
                    assert!(text.contains(&expected));
                    assert!(text.contains("image-fixture"));
                } else {
                    let body: serde_json::Value = serde_json::from_str(&text).unwrap();
                    assert_eq!(body["content"], "edited");
                    match index {
                        1 => assert!(
                            body.get("embeds").is_none(),
                            "a text edit must preserve protected media"
                        ),
                        2 => assert_eq!(body["embeds"][0]["title"], "replacement"),
                        3 => assert_eq!(body["embeds"], serde_json::json!([])),
                        _ => unreachable!(),
                    }
                }
                let body = if index == 1 {
                    r#"{"status":403,"detail":"denied"}"#
                } else {
                    r#"{"id":"m","channel_id":"c","author_id":"me","content":"edited","created_at":"2026-10-23T12:00:00Z"}"#
                };
                let status = if index == 1 { 403 } else { 200 };
                let response = format!(
                    "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                reader
                    .get_mut()
                    .write_all(response.as_bytes())
                    .await
                    .unwrap();
            }
        });
        let api = Api::new(&format!("http://{address}"), Arc::new(Session::default())).unwrap();
        let path =
            std::env::temp_dir().join(format!("embed-upload-{}", crate::cache::new_local_id()));
        std::fs::write(&path, b"image-fixture").unwrap();
        let file = Upload {
            path: path.clone(),
            name: "photo.png".into(),
            mime: "image/png".into(),
            size: 13,
            spoiler: false,
            anonymize: false,
        };
        api.send_message_classified("c", "", None, true, &[file], &[embed])
            .await
            .unwrap();
        assert!(api.edit_message("m", "edited", None).await.is_err());
        api.edit_message(
            "m",
            "edited",
            Some(vec![CustomEmbedInput {
                title: "replacement".into(),
                ..Default::default()
            }]),
        )
        .await
        .unwrap();
        api.edit_message("m", "edited", Some(vec![])).await.unwrap();
        server.await.unwrap();
        std::fs::remove_file(path).unwrap();
    }
}
