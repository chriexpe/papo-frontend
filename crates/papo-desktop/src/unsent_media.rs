//! What your unsent messages' attachments look like (`docs/design.md`
//! §14.3): for each file, what it is, the picture's size, and the picture
//! drawn from the local file, so the row is drawn at its final size before
//! the server has it.
//!
//! Seeded from the tray's tiles at Send (their kind, the size the SDK
//! inspected, the thumbnail already decoded), then sharpened from the
//! staged file at the media box's size. A message restored after a relaunch
//! is read back from staging. Owns only what is drawn; the message and its
//! progress are the SDK's. Held behind an `Arc`, so a view takes it for
//! nothing; each message's entry carries a version that moves with it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use papo_sdk::Uuid;
use papo_sdk_ui::MediaSize;
use rune::Image;

use crate::composer::tray::{Tile, TileKind};

/// One attachment of an unsent message, as it is drawn.
#[derive(Clone)]
pub(crate) struct UnsentFile {
    pub(crate) kind: TileKind,
    /// Its size in bytes as the tray knew it, for when the SDK reports
    /// none (while the file still copies, or once it was sent).
    pub(crate) size_bytes: Option<u64>,
    /// What the picture's size is known as: its size as shown, when the SDK inspected it (it is
    /// never drawn larger, §14.4); else the tile's thumbnail's shape; else nothing (16:9,
    /// letterboxed). The sent message is drawn from the same, so its row keeps its height.
    pub(crate) media_size: MediaSize,
    /// The picture drawn until a sharp one is ready (the tile's thumbnail).
    pub(crate) image: Option<Image>,
    /// The picture decoded at the media box's size; shown once ready.
    pub(crate) sharp: Option<Image>,
}

impl UnsentFile {
    /// A tray tile as its message draws it until the staged file is read.
    pub(crate) fn from_tile(tile: &Tile) -> UnsentFile {
        // A crop or a turn changes the picture sent: its shape is the
        // edit's, and the tile's thumbnail, of the original, is not shown.
        let reshaped = tile.edit.changes_pixels();
        let thumbnail_shape = tile
            .thumbnail
            .as_ref()
            .filter(|_| !reshaped)
            .and_then(Image::size)
            .filter(|[width, height]| *width > 0 && *height > 0)
            .map(|[width_px, height_px]| MediaSize::Shape {
                width_px,
                height_px,
            });
        let media_size = match tile.sent_size_px() {
            Some((width, height)) => MediaSize::exact(width, height),
            None => thumbnail_shape.unwrap_or(MediaSize::Unknown),
        };
        UnsentFile {
            kind: tile.kind.clone(),
            size_bytes: tile.size_bytes,
            media_size,
            image: tile.thumbnail.clone().filter(|_| !reshaped),
            sharp: None,
        }
    }

    /// A file whose kind is not known (read back without its staged
    /// file): drawn as a card under its name.
    pub(crate) fn unknown() -> UnsentFile {
        UnsentFile {
            kind: TileKind::File,
            size_bytes: None,
            media_size: MediaSize::Unknown,
            image: None,
            sharp: None,
        }
    }

    /// The picture to draw: the sharp one once it decoded, else the one it
    /// had; a failed sharp decode keeps that one.
    pub(crate) fn shown(&self) -> Option<&Image> {
        if self.sharp_ready() {
            return self.sharp.as_ref();
        }
        self.image.as_ref()
    }

    /// Whether the sharp picture decoded and shows.
    pub(crate) fn sharp_ready(&self) -> bool {
        self.sharp
            .as_ref()
            .is_some_and(|sharp| sharp.is_ready() && sharp.error().is_none())
    }

    /// Whether it is drawn as a picture rather than a file card.
    pub(crate) fn is_media(&self) -> bool {
        self.kind != TileKind::File
    }
}

/// One message's attachments, in order, and the version they are at.
#[derive(Clone)]
pub(crate) struct Entry {
    /// Moves with every change, for the row's memo.
    pub(crate) version: u64,
    pub(crate) files: Arc<[UnsentFile]>,
}

/// The attachments of each unsent message, by its local identity.
#[derive(Clone, Default)]
pub(crate) struct UnsentMedia {
    entries: Arc<HashMap<Uuid, Entry>>,
    /// The last version handed out.
    last_version: u64,
    /// Messages a queue has listed. Only these are forgotten when a queue
    /// no longer holds them: one just sent is kept until its queue knows
    /// it.
    listed: HashSet<Uuid>,
}

impl UnsentMedia {
    /// Message `local`'s attachments, when known.
    pub(crate) fn get(&self, local: Uuid) -> Option<&Entry> {
        self.entries.get(&local)
    }

    /// Whether message `local` is known.
    pub(crate) fn contains(&self, local: Uuid) -> bool {
        self.entries.contains_key(&local)
    }

    /// Message `local` draws `files`.
    pub(crate) fn insert(&mut self, local: Uuid, files: Vec<UnsentFile>) {
        self.last_version += 1;
        let entry = Entry {
            version: self.last_version,
            files: files.into(),
        };
        Arc::make_mut(&mut self.entries).insert(local, entry);
    }

    /// Attachment `index` of message `local` has its picture decoding at
    /// the box's size; it shows once ready. The box keeps the shape it was
    /// reserved with, so the row never changes height (§9).
    pub(crate) fn sharpened(&mut self, local: Uuid, index: usize, image: Image) {
        let Some(entry) = self.entries.get(&local) else {
            return;
        };
        let mut files = entry.files.to_vec();
        let Some(file) = files.get_mut(index) else {
            return;
        };
        file.sharp = Some(image);
        self.insert(local, files);
    }

    /// A queue listed message `local`.
    pub(crate) fn listed(&mut self, local: Uuid) {
        self.listed.insert(local);
    }

    /// Forgets the listed messages `pending` says no queue holds any more
    /// (sent and gone, or discarded).
    pub(crate) fn forget_gone(&mut self, pending: impl Fn(Uuid) -> bool) {
        let gone: Vec<Uuid> = self
            .listed
            .iter()
            .copied()
            .filter(|local| !pending(*local))
            .collect();
        if gone.is_empty() {
            return;
        }
        for local in &gone {
            self.listed.remove(local);
        }
        Arc::make_mut(&mut self.entries).retain(|local, _| !gone.contains(local));
    }
}

#[cfg(test)]
mod tests {
    //! What is kept, what shows, and the shapes boxes are reserved from.
    use super::*;

    fn photo() -> UnsentFile {
        UnsentFile {
            kind: TileKind::Image,
            size_bytes: Some(1),
            media_size: MediaSize::exact(1600, 900),
            image: None,
            sharp: None,
        }
    }

    #[test]
    fn a_message_just_sent_is_kept_until_its_queue_lists_it() {
        let mut media = UnsentMedia::default();
        let local = Uuid::from_u128(1);
        media.insert(local, vec![photo()]);
        media.forget_gone(|_| false);
        assert!(media.contains(local), "not listed yet");
        media.listed(local);
        media.forget_gone(|other| other == local);
        assert!(media.contains(local), "still queued");
        media.forget_gone(|_| false);
        assert!(!media.contains(local), "gone from the queue");
    }

    #[test]
    fn a_sharp_picture_shows_only_once_decoded_and_moves_the_version() {
        let images = rune::ImageCache::new(Arc::new(|| {}));
        let thumbnail = images.rgba(1, 1, vec![0_u8; 4]);
        let mut media = UnsentMedia::default();
        let local = Uuid::from_u128(1);
        let mut file = photo();
        file.image = Some(thumbnail.clone());
        media.insert(local, vec![file]);
        let first = media.get(local).map(|entry| entry.version);
        // Not an image at all: its decode fails, and the thumbnail stays.
        media.sharpened(local, 0, images.load(vec![1_u8, 2, 3]));
        let entry = media.get(local).expect("kept");
        assert_ne!(Some(entry.version), first);
        let _ = entry.files[0].sharp.as_ref().map(Image::wait);
        assert_eq!(entry.files[0].shown().map(Image::id), Some(thumbnail.id()));
    }
}
