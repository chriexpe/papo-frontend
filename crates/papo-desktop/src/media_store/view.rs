//! What received media's store hands out and takes in: an attachment's key, what a view
//! wants of it, the fetches to start, and what views draw ([`Drawn`], [`MediaView`]). The
//! rules are `media_store`'s.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use papo_sdk::{AccountId, Uuid};
use rune::Image;

/// One piece of received media as one account sees it: an attachment, or an embed's picture or
/// clip that the server relays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MediaKey {
    pub(crate) account: AccountId,
    /// The attachment's UUID, or the embed's.
    pub(crate) id: Uuid,
    pub(crate) of: MediaOf,
}

/// What a [`MediaKey`] names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MediaOf {
    /// A message's attachment: its server thumbnail, then its original.
    Attachment,
    /// An embed's relayed picture (a link card's, a video's poster, a GIF link's) at a
    /// revision: a refetch that may change the picture keeps the embed's UUID, so the revision
    /// (hashed; its SDK source carries it whole) makes the new picture another key.
    EmbedPicture { revision: u64 },
    /// An embed's relayed clip, a post's own video.
    EmbedClip,
}

impl MediaKey {
    /// Attachment `id` as `account` sees it.
    pub(crate) fn attachment(account: AccountId, id: Uuid) -> MediaKey {
        MediaKey {
            account,
            id,
            of: MediaOf::Attachment,
        }
    }

    /// Embed `embed`'s relayed picture at `revision` as `account` sees it.
    pub(crate) fn embed_picture(account: AccountId, embed: Uuid, revision: &str) -> MediaKey {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        revision.hash(&mut hasher);
        MediaKey {
            account,
            id: embed,
            of: MediaOf::EmbedPicture {
                revision: hasher.finish(),
            },
        }
    }

    /// Embed `embed`'s relayed clip as `account` sees it.
    pub(crate) fn embed_clip(account: AccountId, embed: Uuid) -> MediaKey {
        MediaKey {
            account,
            id: embed,
            of: MediaOf::EmbedClip,
        }
    }

    /// Where the whole of a video or audio, or an attachment's original, comes from; none for
    /// an embed picture, whose source carries a revision the key does not.
    pub(crate) fn whole_source(self) -> Option<papo_sdk::MediaSource> {
        match self.of {
            MediaOf::Attachment => Some(papo_sdk::MediaSource::Attachment(self.id)),
            MediaOf::EmbedClip => Some(papo_sdk::MediaSource::EmbedVideo(self.id)),
            MediaOf::EmbedPicture { .. } => None,
        }
    }
}

/// An attachment's ID, when it parses.
pub(crate) fn attachment_id(attachment: &papo_sdk::Attachment) -> Option<Uuid> {
    attachment
        .id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok())
}

/// What a view drawing an attachment asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Want {
    pub(crate) key: MediaKey,
    /// The server's thumbnail's size in pixels, when it has one.
    pub(crate) thumbnail_px: Option<(u32, u32)>,
    /// The box's longer side in points: pictures are decoded to fit it.
    pub(crate) box_pt: f32,
    /// The box's longer side in device pixels: a thumbnail at least this long is enough.
    pub(crate) box_px: f32,
    /// A GIF: it animates from its original.
    pub(crate) gif: bool,
}

/// What a fetch reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Fetch {
    /// The server's thumbnail, decoded at the box's size.
    Thumbnail,
    /// The original, decoded at the box's size.
    Original,
    /// The original, decoded whole so it animates.
    Animation,
}

/// A fetch to start.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Job {
    pub(crate) key: MediaKey,
    pub(crate) fetch: Fetch,
    /// For a box-size decode, the box's longer side in points.
    pub(crate) box_pt: f32,
}

/// What a view draws for an attachment.
#[derive(Clone, Default)]
pub(crate) struct Drawn {
    /// The still picture: the thumbnail, then the original once sharper.
    pub(crate) still: Option<Image>,
    /// The original decoded whole, for a GIF while it animates.
    pub(crate) animated: Option<Image>,
    /// Loading failed; Retry asks again.
    pub(crate) failed: bool,
    /// Moves with every change, for the rows' memos.
    pub(crate) version: u64,
}

impl Drawn {
    /// The picture to draw: the animation when `animate` and it decoded, else the still.
    pub(crate) fn picture(&self, animate: bool) -> Option<&Image> {
        let ready = |image: &&Image| image.is_ready() && image.error().is_none();
        let animated = self.animated.as_ref().filter(|_| animate).filter(ready);
        animated.or_else(|| self.still.as_ref().filter(ready))
    }
}

/// What views read: each attachment's pictures and which GIFs animate.
#[derive(Clone, Default)]
pub(crate) struct MediaView {
    pub(super) drawn: Arc<HashMap<MediaKey, Drawn>>,
    pub(super) animating: Arc<HashSet<MediaKey>>,
}

impl MediaView {
    /// What attachment `key` draws; nothing yet when it was never asked for.
    pub(crate) fn get(&self, key: MediaKey) -> Option<&Drawn> {
        self.drawn.get(&key)
    }

    /// Whether GIF `key` animates now.
    pub(crate) fn animates(&self, key: MediaKey) -> bool {
        self.animating.contains(&key)
    }
}

/// An image's decoded bytes with its mips, from its size, times `frames` for an animation.
pub(crate) fn decoded_bytes(image: &Image, frames: usize) -> u64 {
    let frames = u64::try_from(frames.max(1)).unwrap_or(u64::MAX);
    image.size().map_or(0, |[width, height]| {
        (u64::from(width) * u64::from(height) * 4 * 4 / 3).saturating_mul(frames)
    })
}
