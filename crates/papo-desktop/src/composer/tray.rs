//! One channel's attachment tray (`docs/design.md` §14.2): the files waiting
//! to be sent with the draft, in the album's order, each still copying into
//! the SDK's staging or staged.
//!
//! The tray is part of the SDK draft (`DraftBody.message.attachments`): a
//! staged tile is saved with the text, and a restored draft brings its tiles
//! back. A tile still copying cannot be saved (the SDK refuses a pending
//! file in a draft), so it lives here until its copy ends; a relaunch in
//! between loses it. Owns the tiles and their order; the view is
//! `tray_view`'s, adding and staging the shell's (`shell/attaching.rs`).

use papo_sdk::{AttachmentEdit, OutgoingAttachment, PendingAttachment, StagedAttachment};
use rune::Image;

/// A tile's identity for this run: element keys, focus, and matching a
/// copy's end or a thumbnail to its tile. Never reused within a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct TileId(pub(crate) u64);

/// What a tile draws, from the file's first bytes (`attaching::kind`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TileKind {
    /// A still or animated picture the tile shows as a thumbnail.
    Image,
    /// A GIF: a thumbnail with a "GIF" badge.
    Gif,
    /// A video: its first frame with its length in a badge, once read.
    Video,
    /// Anything else: the extension block, the name and the size.
    File,
}

impl TileKind {
    /// Whether it draws as a picture in the media box (a still, a GIF, a video's first frame)
    /// rather than a file card.
    pub(crate) fn is_boxed(&self) -> bool {
        *self != TileKind::File
    }
}

/// A local video as read for its tile and its unsent message: its first frame on a surface,
/// its length and its size as shown.
#[derive(Clone, Debug)]
pub(crate) struct LocalVideo {
    pub(crate) surface: rune::NativeSurface,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) size_px: Option<[u32; 2]>,
}

/// What the SDK's inspection (`inspect_image`) said of a tile's file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Inspection {
    /// Not yet: the file still copies, or the inspection runs.
    Pending,
    /// A picture the SDK can clean, its size as shown (orientation
    /// applied) in pixels; an animation can't be cropped or turned.
    Cleanable {
        width: u32,
        height: u32,
        animated: bool,
    },
    /// Not an image: it goes as it is, and Anonymize is not offered.
    NotAnImage,
    /// An image the SDK cannot clean (HEIC, TIFF, RAW) or a damaged one:
    /// refused while Anonymize is on.
    Uncleanable,
}

/// Where a tile's bytes stand.
#[derive(Clone)]
pub(crate) enum TileSource {
    /// Still being copied into staging; dropping the last handle stops it.
    Copying(PendingAttachment),
    /// In this account's staging, ready to save and send.
    Staged(StagedAttachment),
    /// A tile the headless tests make: the SDK's handles need a signed-in
    /// session.
    #[cfg(test)]
    Fixture,
}

/// One attached file. Two tiles are equal when they are the same file in
/// the same state (the SDK handle's identity, not its bytes).
#[derive(Clone)]
pub(crate) struct Tile {
    pub(crate) id: TileId,
    /// The file name, without a path; the name it is sent under.
    pub(crate) name: String,
    /// Its size in bytes, once known (a copy says it when it ends).
    pub(crate) size_bytes: Option<u64>,
    pub(crate) kind: TileKind,
    pub(crate) source: TileSource,
    /// The picture decoded at tile size, once its bytes were read.
    pub(crate) thumbnail: Option<Image>,
    /// A video's first frame, length and size, once its bytes were read.
    pub(crate) video: Option<LocalVideo>,
    /// What the SDK said of the staged file.
    pub(crate) inspection: Inspection,
    /// The crop, turns, Random name and Anonymize it is sent with; applied
    /// by the SDK at Send, the staged file untouched until then.
    pub(crate) edit: AttachmentEdit,
    /// The name it had before a random one replaced it, this run: Random
    /// name off brings it back.
    pub(crate) typed_name: Option<String>,
}

impl PartialEq for Tile {
    fn eq(&self, other: &Tile) -> bool {
        let same_source = match (&self.source, &other.source) {
            (TileSource::Copying(a), TileSource::Copying(b)) => a.id() == b.id(),
            (TileSource::Staged(a), TileSource::Staged(b)) => a == b,
            #[cfg(test)]
            (TileSource::Fixture, TileSource::Fixture) => true,
            _ => false,
        };
        self.id == other.id
            && self.name == other.name
            && self.size_bytes == other.size_bytes
            && self.kind == other.kind
            && same_source
            && self.thumbnail.as_ref().map(Image::id) == other.thumbnail.as_ref().map(Image::id)
            && self.video.as_ref().map(|video| video.surface.revision())
                == other.video.as_ref().map(|video| video.surface.revision())
            && self.inspection == other.inspection
            && self.edit == other.edit
            && self.typed_name == other.typed_name
    }
}

impl std::fmt::Debug for Tile {
    /// Never the file's contents; the name is the person's own.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Tile")
            .field("id", &self.id)
            .field("kind", &self.kind)
            .field("size_bytes", &self.size_bytes)
            .field("preparing", &self.preparing())
            .finish_non_exhaustive()
    }
}

impl Tile {
    /// The picture's size as shown, in pixels, once inspected.
    pub(crate) fn size_px(&self) -> Option<(u32, u32)> {
        match self.inspection {
            Inspection::Cleanable { width, height, .. } => Some((width, height)),
            Inspection::Pending | Inspection::NotAnImage | Inspection::Uncleanable => None,
        }
    }

    /// The picture as it will be sent, in pixels: cropped and turned.
    pub(crate) fn sent_size_px(&self) -> Option<(u32, u32)> {
        if let Some(video) = &self.video {
            return video.size_px.map(|[width, height]| (width, height));
        }
        let (width, height) = self.size_px()?;
        if let Some(crop) = self.edit.crop {
            return Some((crop.width, crop.height));
        }
        if self.edit.quarter_turns % 2 == 1 {
            Some((height, width))
        } else {
            Some((width, height))
        }
    }

    /// Whether the SDK cleans this file: a picture it can rewrite.
    pub(crate) fn cleanable(&self) -> bool {
        matches!(self.inspection, Inspection::Cleanable { .. })
    }

    /// Whether it goes anonymized: Anonymize on, and a file the SDK cleans
    /// (with the device default, other files go as they are).
    pub(crate) fn anonymized(&self) -> bool {
        self.edit.anonymize.is_on() && self.cleanable()
    }

    /// Whether it goes under a random name: chosen (any file), or
    /// anonymized.
    pub(crate) fn renamed(&self) -> bool {
        self.anonymized() || self.edit.random_name
    }

    /// Whether the SDK would refuse it at Send: Anonymize on, and an image
    /// it cannot clean.
    pub(crate) fn refused_anonymized(&self) -> bool {
        self.edit.anonymize.is_on() && self.inspection == Inspection::Uncleanable
    }

    /// Its copy still running, if it is.
    pub(crate) fn copying(&self) -> Option<&PendingAttachment> {
        match &self.source {
            TileSource::Copying(pending) => Some(pending),
            TileSource::Staged(_) => None,
            #[cfg(test)]
            TileSource::Fixture => None,
        }
    }

    /// Its staged file, once copied.
    pub(crate) fn staged(&self) -> Option<&StagedAttachment> {
        match &self.source {
            TileSource::Staged(staged) => Some(staged),
            TileSource::Copying(_) => None,
            #[cfg(test)]
            TileSource::Fixture => None,
        }
    }

    /// Whether its copy into staging is still running.
    pub(crate) fn preparing(&self) -> bool {
        matches!(self.source, TileSource::Copying(_))
    }

    /// The tile as the SDK sends it: a staged file, or one still copying
    /// (the message then waits as Preparing).
    pub(crate) fn outgoing(&self) -> OutgoingAttachment {
        match &self.source {
            TileSource::Copying(pending) => {
                OutgoingAttachment::from_pending(self.name.clone(), pending.clone())
                    .with_edit(self.edit)
            }
            TileSource::Staged(staged) => {
                OutgoingAttachment::from_staged(self.name.clone(), staged.clone())
                    .with_edit(self.edit)
            }
            #[cfg(test)]
            TileSource::Fixture => OutgoingAttachment::from_bytes(self.name.clone(), Vec::new()),
        }
    }

    /// The tile as a draft stores it; none while it is still copying.
    pub(crate) fn stored(&self) -> Option<OutgoingAttachment> {
        match &self.source {
            TileSource::Copying(_) => None,
            TileSource::Staged(staged) => Some(
                OutgoingAttachment::from_staged(self.name.clone(), staged.clone())
                    .with_edit(self.edit),
            ),
            #[cfg(test)]
            TileSource::Fixture => Some(OutgoingAttachment::from_bytes(
                self.name.clone(),
                Vec::new(),
            )),
        }
    }
}

/// The tiles of one draft, in order.
#[derive(Clone, Default)]
pub(crate) struct Tray {
    tiles: Vec<Tile>,
}

impl Tray {
    pub(crate) fn tiles(&self) -> &[Tile] {
        &self.tiles
    }

    /// The tiles' identities, in order.
    pub(crate) fn ids(&self) -> Vec<TileId> {
        self.tiles.iter().map(|tile| tile.id).collect()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.tiles.len()
    }

    /// The bytes the tray holds, counting only sizes already known.
    pub(crate) fn known_bytes(&self) -> u64 {
        self.tiles
            .iter()
            .filter_map(|tile| tile.size_bytes)
            .fold(0, u64::saturating_add)
    }

    pub(crate) fn get(&self, id: TileId) -> Option<&Tile> {
        self.tiles.iter().find(|tile| tile.id == id)
    }

    pub(crate) fn get_mut(&mut self, id: TileId) -> Option<&mut Tile> {
        self.tiles.iter_mut().find(|tile| tile.id == id)
    }

    /// Adds `tile` at the end.
    pub(crate) fn push(&mut self, tile: Tile) {
        self.tiles.push(tile);
    }

    /// Removes the tile `id`; returns where it stood, for the focus to stay
    /// there.
    pub(crate) fn remove(&mut self, id: TileId) -> Option<usize> {
        let at = self.place(id)?;
        self.tiles.remove(at);
        Some(at)
    }

    /// Moves the tile `id` by `step` places (−1 left, +1 right), staying
    /// within the tray. Returns whether it moved.
    pub(crate) fn shift(&mut self, id: TileId, step: isize) -> bool {
        let Some(from) = self.place(id) else {
            return false;
        };
        from.checked_add_signed(step)
            .is_some_and(|to| move_item(&mut self.tiles, from, to))
    }

    /// Moves the tile `id` to place `to` (clamped to the last place).
    /// Returns whether the order changed.
    pub(crate) fn move_to(&mut self, id: TileId, to: usize) -> bool {
        let Some(from) = self.place(id) else {
            return false;
        };
        let last = self.tiles.len() - 1;
        move_item(&mut self.tiles, from, to.min(last))
    }

    /// Where the tile `id` stands.
    pub(crate) fn place(&self, id: TileId) -> Option<usize> {
        self.tiles.iter().position(|tile| tile.id == id)
    }

    /// The copy of `id` ended with `staged`: the tile is ready. Returns
    /// whether it was still waiting for it.
    pub(crate) fn staged(&mut self, id: TileId, staged: StagedAttachment) -> bool {
        let Some(tile) = self.get_mut(id) else {
            return false;
        };
        if !tile.preparing() {
            return false;
        }
        tile.size_bytes = Some(staged.size_bytes());
        tile.source = TileSource::Staged(staged);
        true
    }

    /// Empties the tray, handing its tiles over (to a send).
    pub(crate) fn take(&mut self) -> Vec<Tile> {
        std::mem::take(&mut self.tiles)
    }

    /// Puts `tiles` back in front of what is there (a send that failed).
    pub(crate) fn restore(&mut self, tiles: Vec<Tile>) {
        let after = std::mem::replace(&mut self.tiles, tiles);
        self.tiles.extend(after);
    }
}

/// Moves `items[from]` to place `to`; false when `to` is past the end or
/// the same place.
fn move_item<T>(items: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from == to || from >= items.len() || to >= items.len() {
        return false;
    }
    let item = items.remove(from);
    items.insert(to, item);
    true
}

#[cfg(test)]
mod tests {
    //! The order rules; tiles themselves need SDK handles, which the shell
    //! tests make.
    use super::move_item;

    #[test]
    fn an_item_moves_one_place_and_stops_at_the_ends() {
        let mut items = vec![1, 2, 3];
        assert!(move_item(&mut items, 0, 1));
        assert_eq!(items, [2, 1, 3]);
        assert!(!move_item(&mut items, 2, 3));
        assert_eq!(items, [2, 1, 3]);
    }

    #[test]
    fn a_dragged_item_lands_where_it_was_dropped() {
        let mut items = vec![1, 2, 3, 4];
        assert!(move_item(&mut items, 0, 2));
        assert_eq!(items, [2, 3, 1, 4]);
        assert!(move_item(&mut items, 3, 0));
        assert_eq!(items, [4, 2, 3, 1]);
        assert!(!move_item(&mut items, 1, 1));
    }
}
