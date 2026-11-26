//! What a message's link previews show (`docs/design.md` §15.1): which embeds become previews,
//! in what order and how many, a card's parts, a link to a bare GIF drawn as media, and whether
//! a video is the provider's player or a clip the server relays. One anatomy for every kind:
//! each part shows when the embed has it, since the server sends no kind.
//! Owns only the rules, from the wire alone; drawing, the colour's contrast, the words and the
//! Show link previews setting are the client's.
//!
//! Every picture is the server's relay of what it downloaded ([`MediaSource::EmbedImage`]) and
//! every clip its video relay ([`MediaSource::EmbedVideo`]). The only address a preview exposes
//! is the page's own, which the message already links: the embed's media, player, author and
//! icon addresses are never handed out, so showing a preview cannot contact the site.

use papo_sdk::{Embed, EmbedMedia, MediaSource, Message, Uuid};

use crate::attachment_kind::AttachmentKind;
use crate::media_box::{MediaBounds, MediaBox, MediaFit, MediaSize, largest_of_shape, media_box};

/// The most previews a message draws, in link order; a fourth link stays text.
pub const MAX_PREVIEWS: usize = 3;

/// The most fields a card draws, three to a row.
pub const MAX_FIELDS: usize = 6;

/// The shape a card picture without a width and height is reserved at (width over height).
pub const UNSIZED_PICTURE_ASPECT: f32 = 1.91;

/// The shape a provider's player, and so its poster, is reserved at (width over height). The
/// server sends no player ratio.
pub const PLAYER_ASPECT: f32 = 16.0 / 9.0;

/// The server's `fetch_method` for a link straight to a GIF (or a GIF provider's page): its
/// picture is the animated GIF itself.
const FETCH_METHOD_GIF: &str = "gif";

/// One link preview.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkPreview {
    /// A card: the rule, provider line, title, description, fields, picture and footer.
    Card(Box<LinkCard>),
    /// A link to a bare picture or GIF: media at Image size, with no rule and no provider line.
    Media(PreviewMedia),
}

impl LinkPreview {
    /// The embed this preview shows.
    #[must_use]
    pub fn embed(&self) -> Uuid {
        match self {
            LinkPreview::Card(card) => card.embed,
            LinkPreview::Media(media) => media.embed,
        }
    }

    /// The page this preview links to, when it has a safe one.
    #[must_use]
    pub fn page(&self) -> Option<&PageLink> {
        match self {
            LinkPreview::Card(card) => card.page.as_ref(),
            LinkPreview::Media(media) => media.page.as_ref(),
        }
    }
}

/// A card's parts; a part the embed lacks is `None` or empty.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkCard {
    /// The embed's UUID.
    pub embed: Uuid,
    /// The page the title opens.
    pub page: Option<PageLink>,
    /// The embed's own colour for the rule, as sRGB; the client moves it to its contrast floor.
    pub colour: Option<[u8; 3]>,
    /// The provider's or site's name.
    pub provider: Option<String>,
    /// The author's name.
    pub author: Option<String>,
    /// The title, which is the link.
    pub title: Option<String>,
    /// The description.
    pub description: Option<String>,
    /// Up to [`MAX_FIELDS`] fields, in the embed's order.
    pub fields: Vec<CardField>,
    /// The picture, or a video's poster.
    pub picture: Option<CardPicture>,
    /// A video, whose poster is `picture` when the server relayed one.
    pub video: Option<CardVideo>,
    /// The footer's text.
    pub footer: Option<String>,
}

/// A preview's page: the address the message links, and its host for menus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PageLink {
    /// An `http` or `https` address.
    pub address: String,
    /// Its host, never empty.
    pub host: String,
}

/// One field of a card.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CardField {
    /// The label, never empty.
    pub label: String,
    /// The value, never empty.
    pub value: String,
}

/// A picture the server relays.
#[derive(Clone, Debug, PartialEq)]
pub struct CardPicture {
    /// Where its bytes come from: always [`MediaSource::EmbedImage`].
    pub source: MediaSource,
    /// Its size as the embed gives it.
    pub size: MediaSize,
    /// Whether it is a still picture or a GIF.
    pub kind: AttachmentKind,
    /// Its media type as the server stored it (`image/webp`, `image/gif`…), when sent.
    pub mime: Option<String>,
}

/// The video a card shows.
#[derive(Clone, Debug, PartialEq)]
pub enum CardVideo {
    /// The provider's own player, which only a `WebView` can load: the poster stands for it.
    Provider,
    /// A clip the server relays, played like a video file.
    Clip {
        /// Always [`MediaSource::EmbedVideo`].
        source: MediaSource,
        /// Its size, else its poster's.
        size: MediaSize,
        /// Its media type, `video/…`.
        mime: String,
    },
}

/// A link to a bare picture or GIF, drawn as media.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewMedia {
    /// The embed's UUID.
    pub embed: Uuid,
    /// The page or picture address the message links.
    pub page: Option<PageLink>,
    /// The embed's title, for the spoken label.
    pub title: Option<String>,
    /// The picture.
    pub picture: CardPicture,
}

/// The previews `message` draws: its embeds in the server's order (the order of the links in
/// the text), each that has something to show, at most [`MAX_PREVIEWS`].
#[must_use]
pub fn link_previews(message: &Message) -> Vec<LinkPreview> {
    message
        .embeds
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(preview)
        .take(MAX_PREVIEWS)
        .collect()
}

/// The preview `embed` makes, or `None` when the server found nothing to show.
#[must_use]
pub fn preview(embed: &Embed) -> Option<LinkPreview> {
    let id = embed.id.as_deref()?.parse::<Uuid>().ok()?;
    let page = embed.url.as_deref().and_then(page_link);
    let picture = picture(id, embed);
    if embed.fetch_method.as_deref() == Some(FETCH_METHOD_GIF) {
        return Some(LinkPreview::Media(PreviewMedia {
            embed: id,
            page,
            title: text(embed.title.as_deref()),
            picture: picture?,
        }));
    }
    let card = LinkCard {
        embed: id,
        page,
        colour: embed.color.as_deref().and_then(colour),
        provider: text(embed.provider.as_deref()).or_else(|| text(embed.site_name.as_deref())),
        author: text(
            embed
                .author
                .as_ref()
                .and_then(|author| author.name.as_deref()),
        ),
        title: text(embed.title.as_deref()),
        description: text(embed.description.as_deref()),
        fields: fields(embed),
        video: video(id, embed, picture.as_ref()),
        picture,
        footer: text(
            embed
                .footer
                .as_ref()
                .and_then(|footer| footer.text.as_deref()),
        ),
    };
    let shows_something = card.title.is_some()
        || card.description.is_some()
        || !card.fields.is_empty()
        || card.picture.is_some()
        || card.video.is_some();
    shows_something.then(|| LinkPreview::Card(Box::new(card)))
}

/// The box a card's picture or video takes within `bounds` (Embed size's, in the card's
/// column), so the row is reserved before anything loads: a provider's poster is the player's
/// 16 : 9 at the largest; a clip its own size, never enlarged; a picture its own size, never
/// enlarged, else 1.91 : 1 at the largest. Pictures fill their box; a clip of unknown size is
/// letterboxed as a video file is.
#[must_use]
pub fn card_box(card: &LinkCard, bounds: MediaBounds) -> Option<MediaBox> {
    match (&card.video, &card.picture) {
        (Some(CardVideo::Provider), _) => Some(filled(PLAYER_ASPECT, bounds)),
        (Some(CardVideo::Clip { size, .. }), _) => Some(media_box(*size, bounds)),
        (None, Some(picture)) => Some(match picture.size {
            MediaSize::Unknown => filled(UNSIZED_PICTURE_ASPECT, bounds),
            size => media_box(size, bounds),
        }),
        (None, None) => None,
    }
}

fn filled(aspect: f32, bounds: MediaBounds) -> MediaBox {
    let (width_pt, height_pt) = largest_of_shape(aspect, bounds.sane());
    MediaBox {
        width_pt,
        height_pt,
        fit: MediaFit::Fill,
    }
}

fn picture(embed: Uuid, wire: &Embed) -> Option<CardPicture> {
    // The server relays one picture per embed, stored as its thumbnail, and gives its stored
    // type only when it holds the bytes: a thumbnail it only declared (a custom embed whose
    // picture it never downloaded) has none, and would never load.
    let thumbnail = wire.thumbnail.as_ref()?;
    thumbnail
        .mime_type
        .as_deref()
        .filter(|mime| mime.trim().to_ascii_lowercase().starts_with("image/"))?;
    let revision = wire
        .fetched_at
        .clone()
        .or_else(|| wire.created_at.clone())
        .unwrap_or_default();
    let mime = thumbnail
        .mime_type
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|mime| mime.starts_with("image/"));
    let kind = match AttachmentKind::from_mime(mime.as_deref().unwrap_or_default(), None) {
        AttachmentKind::Gif => AttachmentKind::Gif,
        _ => AttachmentKind::Image,
    };
    Some(CardPicture {
        source: MediaSource::EmbedImage { embed, revision },
        size: size(thumbnail),
        kind,
        mime,
    })
}

fn video(embed: Uuid, wire: &Embed, poster: Option<&CardPicture>) -> Option<CardVideo> {
    let video = wire.video.as_ref()?;
    let mime = video
        .mime_type
        .as_deref()
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .filter(|mime| mime.starts_with("video/"));
    // The server sets a type only on a video it relays (`/embeds/{id}/video`); a provider's
    // player page has none.
    let Some(mime) = mime else {
        return Some(CardVideo::Provider);
    };
    let size = match size(video) {
        MediaSize::Unknown => poster.map_or(MediaSize::Unknown, |poster| poster.size),
        size => size,
    };
    Some(CardVideo::Clip {
        source: MediaSource::EmbedVideo(embed),
        size,
        mime,
    })
}

fn size(media: &EmbedMedia) -> MediaSize {
    let side = |value: Option<i64>| value.and_then(|value| u32::try_from(value).ok());
    match (side(media.width), side(media.height)) {
        (Some(width_px), Some(height_px)) => MediaSize::exact(width_px, height_px),
        _ => MediaSize::Unknown,
    }
}

fn fields(embed: &Embed) -> Vec<CardField> {
    let mut fields = embed.fields.clone().unwrap_or_default();
    // A field without a position keeps its place after the placed ones (the sort is stable).
    fields.sort_by_key(|field| field.position.unwrap_or(i64::MAX));
    fields
        .into_iter()
        .filter_map(|field| {
            Some(CardField {
                label: text(field.name.as_deref())?,
                value: text(field.value.as_deref())?,
            })
        })
        .take(MAX_FIELDS)
        .collect()
}

/// `value` trimmed, or `None` when nothing is left.
fn text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn page_link(address: &str) -> Option<PageLink> {
    let url = url::Url::parse(address.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str().filter(|host| !host.is_empty())?.to_owned();
    Some(PageLink {
        address: url.to_string(),
        host,
    })
}

/// `#RRGGBB` as sRGB.
fn colour(value: &str) -> Option<[u8; 3]> {
    let hex = value.trim().strip_prefix('#')?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).ok();
    Some([channel(0)?, channel(2)?, channel(4)?])
}

#[cfg(test)]
mod tests;
