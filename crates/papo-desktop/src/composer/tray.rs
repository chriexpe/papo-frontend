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

use papo_sdk::{OutgoingAttachment, PendingAttachment, StagedAttachment};
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
    /// Anything else, videos included until they can be decoded (D7c): the
    /// extension block, the name and the size.
    File,
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
            }
            TileSource::Staged(staged) => {
                OutgoingAttachment::from_staged(self.name.clone(), staged.clone())
            }
            #[cfg(test)]
            TileSource::Fixture => OutgoingAttachment::from_bytes(self.name.clone(), Vec::new()),
        }
    }

    /// The tile as a draft stores it; none while it is still copying.
    pub(crate) fn stored(&self) -> Option<OutgoingAttachment> {
        match &self.source {
            TileSource::Copying(_) => None,
            TileSource::Staged(staged) => Some(OutgoingAttachment::from_staged(
                self.name.clone(),
                staged.clone(),
            )),
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
