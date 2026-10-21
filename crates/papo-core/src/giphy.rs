//! Provider transport shared by native clients. It never uses Papo credentials.
use serde::{Deserialize, Serialize};
use std::time::Duration;
const API_ROOT: &str = "https://api.giphy.com/v1/gifs";
const RATING: &str = "pg-13";
pub fn api_key() -> Option<&'static str> {
    option_env!("PAPO_GIPHY_API_KEY").filter(|key| {
        let key = key.trim();
        !key.is_empty() && key != "@PAPO_GIPHY_API_KEY@"
    })
}

pub fn available() -> bool {
    api_key().is_some()
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 200
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

pub fn encode_message(id: &str) -> String {
    format!("giphy:{id}")
}

/// Official Papo wire marker for a GIPHY-backed GIF.
pub fn message_id(content: &str) -> Option<&str> {
    let id = content.trim().strip_prefix("giphy:")?;
    valid_id(id).then_some(id)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GifItem {
    pub id: String,
    pub title: String,
    /// Original/highest-resolution GIF used by the fullscreen viewer.
    pub gif_url: String,
    /// Higher-quality bounded rendition used by sent/chat GIFs.
    pub display_url: String,
    /// Grid rendition used by the picker/category tiles.
    pub preview_url: String,
    /// Provider-supplied still image when one exists.
    pub still_url: Option<String>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GifPage {
    pub items: Vec<GifItem>,
    pub more: bool,
}
pub async fn page(query: &str, offset: u32) -> Result<GifPage, String> {
    let key = api_key().ok_or("GIF search and trending are unavailable in this build. Your favorites and recent GIFs are still available.")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    let endpoint = if query.trim().is_empty() {
        "trending"
    } else {
        "search"
    };
    let mut request = client.get(format!("{API_ROOT}/{endpoint}")).query(&[
        ("api_key", key),
        ("rating", RATING),
        ("limit", "24"),
        ("offset", &offset.min(4999).to_string()),
    ]);
    if !query.trim().is_empty() {
        request = request.query(&[("q", query.trim())]);
    }
    let payload: ListResponse<RawGif> = request
        .send()
        .await
        .map_err(|_| "Couldn’t reach GIPHY")?
        .error_for_status()
        .map_err(|_| "GIPHY could not load GIFs. Try again later.")?
        .json()
        .await
        .map_err(|_| "Invalid GIPHY response")?;
    let more = payload.data.len() == 24;
    Ok(GifPage {
        items: payload.data.into_iter().filter_map(convert_item).collect(),
        more,
    })
}
pub async fn item(id: &str) -> Result<GifItem, String> {
    if !valid_id(id) {
        return Err("Invalid GIF identifier".into());
    }
    let key = api_key().ok_or("GIPHY is not configured for this build")?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;
    #[derive(Deserialize)]
    struct Single {
        data: RawGif,
    }
    let payload: Single = client
        .get(format!("{API_ROOT}/{id}"))
        .query(&[("api_key", key)])
        .send()
        .await
        .map_err(|_| "Couldn’t reach GIPHY")?
        .error_for_status()
        .map_err(|_| "GIF is unavailable")?
        .json()
        .await
        .map_err(|_| "Invalid GIPHY response")?;
    convert_item(payload.data).ok_or("GIF is unavailable".into())
}
fn convert_item(raw: RawGif) -> Option<GifItem> {
    if !valid_id(&raw.id) {
        return None;
    }

    let full = raw
        .images
        .original
        .as_ref()
        .or(raw.images.downsized.as_ref())
        .or(raw.images.fixed_width.as_ref())?;
    // GIPHY documents fixed_width_small as a 100px "nano" rendition. Our
    // two-column picker tiles are roughly 200px wide, so using it first caused
    // visible upscaling/blurring. fixed_width is the intended grid rendition.
    let preview = raw
        .images
        .fixed_width
        .as_ref()
        .or(raw.images.downsized.as_ref())
        .unwrap_or(full);

    // Once selected, GIPHY recommends a higher-quality downsized rendition for
    // chat/messaging. Prefer the <=5MB medium tier here: substantially sharper
    // than the 100/200px grid renditions without forcing every visible message
    // to fetch/decode the original asset.
    let display = raw
        .images
        .downsized_medium
        .as_ref()
        .or(raw.images.downsized.as_ref())
        .or(raw.images.downsized_large.as_ref())
        .unwrap_or(full);

    if !crate::preview::safe_remote_url(&full.url)
        || !crate::preview::safe_remote_url(&display.url)
        || !crate::preview::safe_remote_url(&preview.url)
    {
        return None;
    }

    let still_url = raw
        .images
        .fixed_width_still
        .as_ref()
        .or(raw.images.original_still.as_ref())
        .filter(|image| crate::preview::safe_remote_url(&image.url))
        .map(|image| image.url.clone());

    Some(GifItem {
        id: raw.id,
        title: if raw.title.trim().is_empty() {
            "GIF".into()
        } else {
            raw.title
        },
        gif_url: full.url.clone(),
        display_url: display.url.clone(),
        preview_url: preview.url.clone(),
        still_url,
        width: parse_dimension(&full.width).max(1),
        height: parse_dimension(&full.height).max(1),
    })
}

fn parse_dimension(value: &str) -> u32 {
    value.parse().unwrap_or(0)
}

#[derive(Deserialize)]
struct ListResponse<T> {
    #[serde(default)]
    data: Vec<T>,
}

#[derive(Default, Deserialize)]
struct RawGif {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    images: RawImages,
}

#[derive(Default, Deserialize)]
struct RawImages {
    #[serde(default)]
    original: Option<RawImage>,
    #[serde(default)]
    downsized: Option<RawImage>,
    #[serde(default)]
    downsized_large: Option<RawImage>,
    #[serde(default)]
    downsized_medium: Option<RawImage>,
    #[serde(default)]
    fixed_width: Option<RawImage>,
    #[serde(default)]
    fixed_width_still: Option<RawImage>,
    #[serde(default)]
    original_still: Option<RawImage>,
}

#[derive(Deserialize)]
struct RawImage {
    #[serde(default)]
    url: String,
    #[serde(default)]
    width: String,
    #[serde(default)]
    height: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_markers_are_bounded_and_do_not_match_prose_or_urls() {
        assert_eq!(message_id(&encode_message("abc_123-X")), Some("abc_123-X"));
        assert!(message_id("hello giphy:abc").is_none());
        assert!(message_id("giphy:../secret").is_none());
        assert!(!valid_id(&"x".repeat(201)));
    }
    #[test]
    fn provider_renditions_preserve_dimensions_and_reject_unsafe_urls() {
        let raw = serde_json::json!({"id":"abc","title":"Wave","images": {
            "original":{"url":"https://media.giphy.com/media/abc/giphy.gif","width":"640","height":"480"},
            "fixed_width":{"url":"https://media.giphy.com/media/abc/200w.gif","width":"200","height":"150"},
            "downsized_medium":{"url":"https://media.giphy.com/media/abc/medium.gif","width":"640","height":"480"}
        }});
        let gif = convert_item(serde_json::from_value(raw.clone()).unwrap()).unwrap();
        assert_eq!((gif.width, gif.height), (640, 480));
        assert!(gif.preview_url.ends_with("200w.gif"));
        assert!(gif.display_url.ends_with("medium.gif"));
        let mut invalid = raw;
        invalid["images"]["original"]["url"] = serde_json::json!("file:///etc/passwd");
        assert!(convert_item(serde_json::from_value(invalid).unwrap()).is_none());
    }
}
