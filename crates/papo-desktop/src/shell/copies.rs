//! Following each tray tile's copy into the SDK's staging (`docs/design.md`
//! §14.2): new tiles joining the tray, a watch per copy that says when it
//! ends, and what ends a watch (the tile removed, its draft gone), so a copy
//! nobody waits for stops.
//!
//! Owns the watches; starting a copy is `attaching`'s, the tray the
//! composer's.

use std::collections::HashMap;

use papo_sdk::{PendingAttachment, StagedAttachment, StagingFailure};
use rune::Tx;
use tokio::sync::oneshot;

use super::Shell;
use crate::attaching::{self, Offered, Refused};
use crate::composer::attached::Closed;
use crate::composer::tray::{Tile, TileId, TileKind, TileSource};
use crate::notice::Refusal;
use crate::sdk_bridge::Updates;
use crate::strings;
use crate::timeline::TimelineKey;

/// A copy started for a new tile.
pub(super) struct Started {
    /// The file's name; empty for a pasted picture, named when it joins.
    pub(super) name: String,
    pub(super) size_bytes: u64,
    pub(super) kind: TileKind,
    pub(super) pending: PendingAttachment,
    /// Picture bytes already in memory (a paste), decoded at once.
    pub(super) picture: Option<Vec<u8>>,
}

/// The watch on each copy still running, by tile: dropping a tile's
/// sender ends its watch, which lets go of the copy so it stops.
#[derive(Default)]
pub(crate) struct CopyWatches(HashMap<TileId, oneshot::Sender<()>>);

impl CopyWatches {
    fn add(&mut self, id: TileId) -> oneshot::Receiver<()> {
        let (stop, stopped) = oneshot::channel();
        self.0.insert(id, stop);
        stopped
    }

    fn end(&mut self, id: TileId) {
        self.0.remove(&id);
    }
}

/// Hands copies started for `key` to the shell.
pub(super) fn started_copies(
    updates: &Updates<Shell>,
    key: TimelineKey,
    started: Vec<Started>,
    refused: Option<Refused>,
) {
    if started.is_empty() && refused.is_none() {
        return;
    }
    updates.send_tx(move |shell, tx| shell.copies_started(key, started, refused, tx));
}

impl Shell {
    /// Copies started for `key`'s tray: their tiles join it, each watched
    /// until its copy ends. The tray is checked again (another add may have
    /// filled it meanwhile, or an edit begun), and what no longer fits is
    /// dropped, which stops its copy.
    fn copies_started(
        &mut self,
        key: TimelineKey,
        started: Vec<Started>,
        refused: Option<Refused>,
        tx: &mut Tx,
    ) -> bool {
        let (held, held_bytes) = self.composer.tray_load(key);
        let offered: Vec<Offered> = started
            .iter()
            .map(|file| Offered::File {
                name: file.name.clone(),
                size_bytes: file.size_bytes,
            })
            .collect();
        let plan = attaching::plan(&offered, held, held_bytes);
        let refused = refused.or(plan.refused);
        let mut tiles = Vec::new();
        let mut pictures = Vec::new();
        for (index, file) in started.into_iter().enumerate() {
            if !plan.taken.contains(&index) {
                // Dropped here, its copy stops.
                continue;
            }
            let name = if file.name.is_empty() {
                attaching::pasted_name(strings::attach_pasted_name(), |name| {
                    self.composer.tray_has_name(key, name)
                        || tiles.iter().any(|tile: &Tile| tile.name == name)
                })
            } else {
                file.name
            };
            let id = self.composer.next_tile_id();
            if let Some(png) = file.picture {
                pictures.push((id, png));
            }
            tiles.push(Tile {
                id,
                name,
                size_bytes: Some(file.size_bytes),
                kind: file.kind,
                source: TileSource::Copying(file.pending),
                thumbnail: None,
            });
        }
        let copying: Vec<(TileId, PendingAttachment)> = tiles
            .iter()
            .filter_map(|tile| tile.copying().map(|pending| (tile.id, pending.clone())))
            .collect();
        match self.composer.attach(key, tiles) {
            Ok(effects) => {
                self.run_composer(effects, Some(tx));
                for (id, pending) in copying {
                    self.watch_copy(key, id, pending);
                }
                for (id, png) in pictures {
                    self.thumbnail_read(key, id, png, tx);
                }
            }
            // An edit began while the files were read: nothing is taken.
            Err(Closed::Editing) => {
                self.refuse(Refusal::FinishEdit, tx);
                return true;
            }
            Err(Closed::NoDraft) => return true,
        }
        if let Some(refused) = refused {
            self.refuse(Refusal::Attach(refused), tx);
        }
        true
    }

    /// Waits for the copy of tile `id` to end, until the tile goes. A
    /// copy already over answers at once.
    pub(super) fn watch_copy(&mut self, key: TimelineKey, id: TileId, pending: PendingAttachment) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        let stopped = self.copy_watches.add(id);
        let updates = sdk.updates();
        sdk.run(move |_client| async move {
            let ended = tokio::select! {
                ended = pending.staged() => ended,
                // The tile went: its copy is not waited for, and this
                // handle goes with the watch so the copy can stop.
                _ = stopped => return,
            };
            drop(pending);
            updates.send_tx(move |shell, tx| shell.copy_ended(key, id, ended, tx));
        });
    }

    /// Tiles a refused send put back in `key`'s tray: the ones still
    /// copying are watched again (their watch ended when Send took them).
    pub(super) fn watch_returned(&mut self, key: TimelineKey, tiles: &[Tile]) {
        for tile in tiles {
            if let Some(pending) = tile.copying() {
                self.watch_copy(key, tile.id, pending.clone());
            }
        }
    }

    /// Ends the watches of tiles no draft holds any more (an account
    /// removed with its drafts), so their copies stop.
    pub(super) fn forget_lost_copies(&mut self) {
        let held = self.composer.all_tile_ids();
        self.copy_watches.0.retain(|id, _| held.contains(id));
    }

    /// The copy of tile `id` ended: the tile is ready, or it leaves the
    /// tray and the notice says why.
    fn copy_ended(
        &mut self,
        key: TimelineKey,
        id: TileId,
        ended: Result<StagedAttachment, StagingFailure>,
        tx: &mut Tx,
    ) -> bool {
        self.copy_watches.end(id);
        let Some(tile) = self.composer.tile(key, id) else {
            // Sent meanwhile, or removed: the message holds the copy now.
            return false;
        };
        let name = tile.name.clone();
        let picture = tile.kind != TileKind::File && tile.thumbnail.is_none();
        match ended {
            Ok(staged) => {
                let effects = self.composer.tile_staged(key, id, staged.clone());
                self.run_composer(effects, Some(tx));
                if picture {
                    self.read_thumbnails(key, vec![(id, staged)]);
                }
            }
            Err(failure) => {
                self.take_tile_out(key, id, tx);
                match failure {
                    StagingFailure::TooLarge => {
                        self.refuse(Refusal::Attach(Refused::TooLarge { name }), tx);
                    }
                    StagingFailure::Cancelled => {}
                    _ => self.refuse(Refusal::Attach(Refused::Unreadable { name }), tx),
                }
            }
        }
        true
    }

    /// ✕ on a tile, Delete or Backspace, or the menu's Remove: the tile goes
    /// at once (§14.2: nothing asks, attaching again is the undo). Returns
    /// the tile now in its place, for the keyboard focus.
    pub(crate) fn remove_tile(&mut self, id: TileId, tx: &mut Tx) -> Option<TileId> {
        let key = self.composer.shown()?.0;
        self.take_tile_out(key, id, tx)
    }

    /// Takes tile `id` out of `key`'s tray: its copy stops, the draft is
    /// saved, and the keyboard, if it was on it, moves to the tile now in
    /// its place.
    fn take_tile_out(&mut self, key: TimelineKey, id: TileId, tx: &mut Tx) -> Option<TileId> {
        self.copy_watches.end(id);
        let (next, effects) = self.composer.detach(key, id);
        self.run_composer(effects, Some(tx));
        if self.tray_ui.current == Some(id) {
            self.tray_ui.current = next;
        }
        let shown = self.composer.shown().map(|(_, draft)| draft.tray.ids());
        self.tray_ui.retain(&shown.unwrap_or_default());
        next
    }
}
