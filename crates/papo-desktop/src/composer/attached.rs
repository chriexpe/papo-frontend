//! What happens to a channel's tray (`docs/design.md` §14.2): files added,
//! a copy that ended, a tile removed or moved, a thumbnail decoded. Each
//! change that the stored draft must hold saves it at once; nothing here
//! talks to the SDK.

use papo_sdk::{AttachmentEdit, StagedAttachment};
use rune::Image;

use super::tray::{Inspection, Tile, TileId};
use super::{Composer, DraftState, Effect, TimelineKey};

/// Why files cannot be attached to the shown draft now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Closed {
    /// No text channel shows, or its draft still loads.
    NoDraft,
    /// A message is being edited: attachments are never edited (§11), so
    /// the edit must end first (§14.1).
    Editing,
}

impl Composer {
    /// Whether files can be attached to the shown draft, and which draft.
    pub(crate) fn attach_target(&self) -> Result<TimelineKey, Closed> {
        let (key, draft) = self.shown().ok_or(Closed::NoDraft)?;
        if draft.state == DraftState::Loading {
            return Err(Closed::NoDraft);
        }
        if draft.mode.editing().is_some() {
            return Err(Closed::Editing);
        }
        Ok(key)
    }

    /// A tray tile identity not used before in this run.
    pub(crate) fn next_tile_id(&mut self) -> TileId {
        self.tile_ids += 1;
        TileId(self.tile_ids)
    }

    /// Tile `id` of `key`'s tray, while it is there.
    pub(crate) fn tile(&self, key: TimelineKey, id: TileId) -> Option<&Tile> {
        self.drafts.get(&key)?.tray.get(id)
    }

    /// The tray of `key`'s draft: what it holds and how many bytes known.
    pub(crate) fn tray_load(&self, key: TimelineKey) -> (usize, u64) {
        self.drafts
            .get(&key)
            .map_or((0, 0), |draft| (draft.tray.len(), draft.tray.known_bytes()))
    }

    /// Whether `key`'s tray already holds a file named `name`.
    pub(crate) fn tray_has_name(&self, key: TimelineKey, name: &str) -> bool {
        self.drafts
            .get(&key)
            .is_some_and(|draft| draft.tray.tiles().iter().any(|tile| tile.name == name))
    }

    /// `tiles` join the end of `key`'s tray. Staged ones are saved with the
    /// draft now; one still copying is saved when its copy ends. Refused
    /// (and the tiles dropped, which stops their copies) when the draft is
    /// gone or a message is being edited in it.
    pub(crate) fn attach(
        &mut self,
        key: TimelineKey,
        tiles: Vec<Tile>,
    ) -> Result<Vec<Effect>, Closed> {
        let draft = self.drafts.get_mut(&key).ok_or(Closed::NoDraft)?;
        if draft.mode.editing().is_some() {
            return Err(Closed::Editing);
        }
        let any_staged = tiles.iter().any(|tile| !tile.preparing());
        for tile in tiles {
            draft.tray.push(tile);
        }
        Ok(if any_staged {
            self.tray_changed(key)
        } else {
            Vec::new()
        })
    }

    /// Saves `key`'s draft now if it holds what the stored one does not
    /// (files attached while it loaded).
    pub(crate) fn save_unsaved(&mut self, key: TimelineKey) -> Vec<Effect> {
        self.save(key).into_iter().collect()
    }

    /// Every tile of every draft: a copy is watched only while its tile
    /// is in one.
    pub(crate) fn all_tile_ids(&self) -> std::collections::HashSet<TileId> {
        self.drafts
            .values()
            .flat_map(|draft| draft.tray.ids())
            .collect()
    }

    /// The copy of tile `id` in `key`'s tray ended with `staged`: the tile
    /// is ready and the draft holds it from now on.
    pub(crate) fn tile_staged(
        &mut self,
        key: TimelineKey,
        id: TileId,
        staged: StagedAttachment,
    ) -> Vec<Effect> {
        let ready = self
            .drafts
            .get_mut(&key)
            .is_some_and(|draft| draft.tray.staged(id, staged));
        if ready {
            self.tray_changed(key)
        } else {
            Vec::new()
        }
    }

    /// Removes tile `id` from `key`'s tray (✕, Delete, the menu's Remove, or
    /// a copy that failed). Returns the tile now at its place, for the focus,
    /// and the draft's save. Dropping a tile still copying stops its copy.
    pub(crate) fn detach(&mut self, key: TimelineKey, id: TileId) -> (Option<TileId>, Vec<Effect>) {
        let Some(draft) = self.drafts.get_mut(&key) else {
            return (None, Vec::new());
        };
        let Some(at) = draft.tray.remove(id) else {
            return (None, Vec::new());
        };
        let tiles = draft.tray.tiles();
        let next = tiles
            .get(at)
            .or_else(|| at.checked_sub(1).and_then(|before| tiles.get(before)))
            .map(|tile| tile.id);
        (next, self.tray_changed(key))
    }

    /// Moves tile `id` of `key`'s tray `step` places (Alt+←/→).
    pub(crate) fn shift_tile(&mut self, key: TimelineKey, id: TileId, step: isize) -> Vec<Effect> {
        let moved = self
            .drafts
            .get_mut(&key)
            .is_some_and(|draft| draft.tray.shift(id, step));
        if moved {
            self.tray_changed(key)
        } else {
            Vec::new()
        }
    }

    /// Moves tile `id` of `key`'s tray to place `to` (a drag's drop).
    pub(crate) fn move_tile(&mut self, key: TimelineKey, id: TileId, to: usize) -> Vec<Effect> {
        let moved = self
            .drafts
            .get_mut(&key)
            .is_some_and(|draft| draft.tray.move_to(id, to));
        if moved {
            self.tray_changed(key)
        } else {
            Vec::new()
        }
    }

    /// Tile `id` of `key`'s tray has its picture, decoding at tile size.
    /// Returns whether a tile took it.
    pub(crate) fn tile_thumbnail(
        &mut self,
        key: TimelineKey,
        id: TileId,
        thumbnail: Image,
    ) -> bool {
        let Some(tile) = self.tile_mut(key, id) else {
            return false;
        };
        tile.thumbnail = Some(thumbnail);
        true
    }

    /// The SDK inspected tile `id` of `key`'s tray. Returns the tile, when
    /// it is still there.
    pub(crate) fn tile_inspected(
        &mut self,
        key: TimelineKey,
        id: TileId,
        inspection: Inspection,
    ) -> Option<&Tile> {
        let tile = self.tile_mut(key, id)?;
        tile.inspection = inspection;
        Some(tile)
    }

    /// The editor saved tile `id` of `key`'s tray: its name and its edit.
    /// The draft is saved.
    pub(crate) fn tile_edited(
        &mut self,
        key: TimelineKey,
        id: TileId,
        name: String,
        edit: AttachmentEdit,
    ) -> Vec<Effect> {
        let Some(tile) = self.tile_mut(key, id) else {
            return Vec::new();
        };
        if tile.name == name && tile.edit == edit {
            return Vec::new();
        }
        tile.name = name;
        tile.edit = edit;
        self.tray_changed(key)
    }

    /// The name tile `id` of `key`'s tray had before a random one, this run.
    pub(crate) fn tile_typed_name(&mut self, key: TimelineKey, id: TileId, typed: Option<String>) {
        if let Some(tile) = self.tile_mut(key, id) {
            tile.typed_name = typed;
        }
    }

    fn tile_mut(&mut self, key: TimelineKey, id: TileId) -> Option<&mut Tile> {
        self.drafts
            .get_mut(&key)
            .and_then(|draft| draft.tray.get_mut(id))
    }

    /// `key`'s tray changed what the stored draft holds: it is saved now,
    /// during an edit too (`save` keeps the field set aside). A draft
    /// still loading is saved once loaded (`save_unsaved`).
    fn tray_changed(&mut self, key: TimelineKey) -> Vec<Effect> {
        let Some(draft) = self.drafts.get_mut(&key) else {
            return Vec::new();
        };
        if draft.state == DraftState::Loading {
            return Vec::new();
        }
        draft.state = DraftState::Unsaved { due_s: 0.0 };
        self.save(key).into_iter().collect()
    }
}
