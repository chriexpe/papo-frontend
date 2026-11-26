//! A message's link previews (`docs/design.md` §15.1–§15.3, D8a): which it draws (the SDK's
//! anatomy, none with Show link previews off), where each one's picture or clip sits among the
//! message's media, and what a press on a card asks for. Drawing is `view`'s.
//!
//! A message's media are addressed by one place (`MediaAt::index`): its attachments first, then
//! the pictures of its previews, so the viewer, the media menu, Retry and saving reach a card's
//! picture as they reach an attachment. Every picture and clip is the server's relay; the only
//! address a card opens is the page's, through §9's link rules.

use papo_sdk::{AccountId, MediaSource, Message, Uuid};
use papo_sdk_ui::{
    CardPicture, CardVideo, LinkPreview, MediaBounds, MediaBox, ProviderPlayer, card_box, media_box,
};

use crate::media_store::MediaKey;
use crate::message_media::received::MediaAt;
use crate::provider_players::PlayerKey;

pub(crate) mod player;
pub(crate) mod view;

/// What a press on a card asks for, besides its picture's (which are media presses) and its
/// title's (a link press).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CardPress {
    /// The card's menu, beside the pointer at `at` (window points).
    Menu([f32; 2]),
    /// A provider video's play disc where it never plays (pins, search, a confirmation): the
    /// page opens in the browser, by §9's link rules.
    OpenInBrowser,
}

/// The previews `message` draws: none with Show link previews off (`shown`), so nothing is drawn
/// or reserved.
pub(crate) fn previews(message: &Message, shown: bool) -> Vec<LinkPreview> {
    if !shown {
        return Vec::new();
    }
    papo_sdk_ui::link_previews(message)
}

/// The number of `message`'s attachments: its previews' places follow them.
pub(crate) fn attachment_count(message: &Message) -> usize {
    message.attachments.as_deref().map_or(0, <[_]>::len)
}

/// One piece of a message's media, by its place.
#[derive(Clone)]
pub(crate) enum MediaItem {
    /// An attachment (the place is one of its attachments').
    Attachment,
    /// A link preview.
    Preview(Box<LinkPreview>),
}

/// The media item at place `index` of `message`: an attachment, else the preview after them.
pub(crate) fn media_item(message: &Message, index: usize, shown: bool) -> Option<MediaItem> {
    let attachments = message.attachments.as_deref().unwrap_or_default();
    if index < attachments.len() {
        return Some(MediaItem::Attachment);
    }
    let preview = previews(message, shown)
        .into_iter()
        .nth(index - attachments.len())?;
    Some(MediaItem::Preview(Box::new(preview)))
}

/// A preview's picture: a card's (its poster for a video), or a GIF link's.
pub(crate) fn picture_of(preview: &LinkPreview) -> Option<&CardPicture> {
    match preview {
        LinkPreview::Card(card) => card.picture.as_ref(),
        LinkPreview::Media(media) => Some(&media.picture),
    }
}

/// A card's provider video, when it is the provider's player (not a clip the server relays).
pub(crate) fn is_provider_video(preview: &LinkPreview) -> bool {
    matches!(preview, LinkPreview::Card(card) if matches!(card.video, Some(CardVideo::Provider(_))))
}

/// A card's provider player, when its video is an allowlisted provider's (the SDK's
/// descriptor): the player's key at `at` and the descriptor.
pub(crate) fn player_of(
    at: MediaAt,
    preview: &LinkPreview,
) -> Option<(PlayerKey, &ProviderPlayer)> {
    let LinkPreview::Card(card) = preview else {
        return None;
    };
    let Some(CardVideo::Provider(Some(player))) = &card.video else {
        return None;
    };
    let key = PlayerKey {
        account: at.account,
        channel: at.channel,
        message: at.message,
        embed: card.embed,
    };
    Some((key, player))
}

/// A preview's relayed clip: its key and its source.
pub(crate) fn clip_of(account: AccountId, preview: &LinkPreview) -> Option<(MediaKey, &CardVideo)> {
    let LinkPreview::Card(card) = preview else {
        return None;
    };
    match &card.video {
        Some(video @ CardVideo::Clip { .. }) => {
            Some((MediaKey::embed_clip(account, card.embed), video))
        }
        _ => None,
    }
}

/// The store's key of a preview's picture, and the exact SDK source it is fetched from.
pub(crate) fn picture_key(
    account: AccountId,
    preview: &LinkPreview,
) -> Option<(MediaKey, MediaSource)> {
    let (embed, revision) = relayed_picture(preview)?;
    Some((
        MediaKey::embed_picture(account, embed, revision),
        MediaSource::EmbedImage {
            embed,
            revision: revision.to_owned(),
        },
    ))
}

/// The embed whose relayed picture a preview draws, and that picture's revision.
fn relayed_picture(preview: &LinkPreview) -> Option<(Uuid, &str)> {
    match &picture_of(preview)?.source {
        MediaSource::EmbedImage { embed, revision } => Some((*embed, revision.as_str())),
        _ => None,
    }
}

/// What a preview is spoken and saved as: its title, else its page's host, else nothing.
pub(crate) fn name_of(preview: &LinkPreview) -> String {
    let title = match preview {
        LinkPreview::Card(card) => card.title.as_deref(),
        LinkPreview::Media(media) => media.title.as_deref(),
    };
    title
        .or_else(|| preview.page().map(|page| page.host.as_str()))
        .unwrap_or_default()
        .to_owned()
}

/// The provider a card names: its provider or site, else its page's host.
pub(crate) fn provider_of(preview: &LinkPreview) -> String {
    let named = match preview {
        LinkPreview::Card(card) => card.provider.as_deref(),
        LinkPreview::Media(_) => None,
    };
    named
        .or_else(|| preview.page().map(|page| page.host.as_str()))
        .unwrap_or_default()
        .to_owned()
}

/// The embed a preview's picture belongs to, and its revision, for a save.
pub(crate) fn saved_source(preview: &LinkPreview) -> Option<(Uuid, String)> {
    relayed_picture(preview).map(|(embed, revision)| (embed, revision.to_owned()))
}

/// The gap between a card's rule and its parts (§15.1): its density's lead gap (10 pt at
/// Normal), whatever the row's lead is (none when Show avatars and Show times are both off).
pub(crate) fn rule_gap_pt(look: &crate::design::Look, density: crate::design::Density) -> f32 {
    look.spacing(density).lead_gap_pt
}

/// The bounds of a card's picture: Embed size's, less the rule and the lead gap beside it, so
/// the card is as wide as Embed size.
pub(crate) fn picture_bounds(embed_bounds: MediaBounds, lead_gap_pt: f32) -> MediaBounds {
    MediaBounds {
        max_width_pt: embed_bounds.max_width_pt - crate::design::CARD_RULE_PT - lead_gap_pt,
        max_height_pt: embed_bounds.max_height_pt,
    }
}

/// The box a preview's picture (or poster) is drawn in: a card's within `embed_bounds` (with
/// the density's `lead_gap_pt`), a GIF link's within `image_bounds`; none without one, or for a
/// clip, which plays as a video.
pub(crate) fn picture_box(
    preview: &LinkPreview,
    embed_bounds: MediaBounds,
    image_bounds: MediaBounds,
    lead_gap_pt: f32,
) -> Option<MediaBox> {
    match preview {
        LinkPreview::Card(card) => match card.video {
            Some(CardVideo::Clip { .. }) => None,
            _ => card_box(card, picture_bounds(embed_bounds, lead_gap_pt)),
        },
        LinkPreview::Media(media) => Some(media_box(media.picture.size, image_bounds)),
    }
}

/// The box a preview's relayed clip plays in, within `embed_bounds` (with the density's
/// `lead_gap_pt`); none for a preview without one.
pub(crate) fn clip_box(
    preview: &LinkPreview,
    embed_bounds: MediaBounds,
    lead_gap_pt: f32,
) -> Option<MediaBox> {
    let LinkPreview::Card(card) = preview else {
        return None;
    };
    match card.video {
        Some(CardVideo::Clip { .. }) => card_box(card, picture_bounds(embed_bounds, lead_gap_pt)),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
