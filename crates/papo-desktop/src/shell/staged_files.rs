//! Reading staged files back for the tray (`docs/design.md` §14.2): a
//! stored draft's tiles after a switch or a relaunch, and each picture
//! tile's thumbnail, decoded by Rune at tile size off the UI thread.
//!
//! Owns reading through `open_staged`; the tiles are the composer's.

use std::io::Read;

use papo_sdk::{OutgoingAttachment, Session, StagedAttachment};
use rune::Tx;

use super::Shell;
use crate::attaching::{self, HEAD_BYTES, THUMBNAIL_SOURCE_MAX_BYTES};
use crate::composer::tray::{Tile, TileId, TileKind, TileSource};
use crate::timeline::TimelineKey;

/// The side a tile's picture is decoded to fit, in points: twice the tile,
/// so a picture up to 2 : 1 still covers the 72 pt square it is cropped to.
const TILE_DECODE_PT: f32 = 144.0;

/// A stored draft's file, read back for its tile.
pub(crate) struct Restored {
    name: String,
    staged: StagedAttachment,
    kind: TileKind,
}

impl Shell {
    /// A tile for a stored draft's file, with a fresh identity.
    pub(super) fn restored_tile(&mut self, file: Restored) -> Tile {
        Tile {
            id: self.composer.next_tile_id(),
            name: file.name,
            size_bytes: Some(file.staged.size_bytes()),
            kind: file.kind,
            source: TileSource::Staged(file.staged),
            thumbnail: None,
        }
    }

    /// Reads the staged bytes of `pictures` in `key`'s tray and decodes
    /// each at tile size.
    pub(super) fn read_thumbnails(
        &mut self,
        key: TimelineKey,
        pictures: Vec<(TileId, StagedAttachment)>,
    ) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        for (id, staged) in pictures {
            let updates = sdk.updates();
            sdk.run(move |client| async move {
                let Ok(session) = client.session(key.account) else {
                    return;
                };
                match read_staged(&session, &staged, THUMBNAIL_SOURCE_MAX_BYTES).await {
                    Some(bytes) => {
                        updates.send_tx(move |shell, tx| shell.thumbnail_read(key, id, bytes, tx));
                    }
                    None => log::debug!("a tile's picture was not read"),
                }
            });
        }
    }

    /// A tile's picture bytes are in: Rune decodes them at tile size off the
    /// UI thread, and the tile draws them once ready.
    pub(super) fn thumbnail_read(
        &mut self,
        key: TimelineKey,
        id: TileId,
        bytes: Vec<u8>,
        tx: &mut Tx,
    ) -> bool {
        let Some(images) = &self.images else {
            return false;
        };
        let image = images.load_scaled(bytes, TILE_DECODE_PT, tx.scale());
        self.composer.tile_thumbnail(key, id, image)
    }
}

/// Reads a stored draft's files back for their tiles: each staged file's
/// kind from its first bytes. A file the SDK no longer has is left out.
pub(super) async fn restored_files(
    session: &Session,
    attachments: Vec<OutgoingAttachment>,
) -> Vec<Restored> {
    let mut restored = Vec::new();
    for attachment in attachments {
        let Some(staged) = attachment.staged().cloned() else {
            continue;
        };
        let Some(head) = read_staged(session, &staged, HEAD_BYTES as u64).await else {
            log::warn!("a draft's file could not be read back");
            continue;
        };
        restored.push(Restored {
            kind: attaching::kind_of(&head, staged.size_bytes()),
            name: attachment.filename,
            staged,
        });
    }
    restored
}

/// The tiles among `tiles` that draw a picture, with their staged files.
pub(super) fn pictures(tiles: &[Tile]) -> Vec<(TileId, StagedAttachment)> {
    tiles
        .iter()
        .filter(|tile| tile.kind != TileKind::File)
        .filter_map(|tile| tile.staged().map(|staged| (tile.id, staged.clone())))
        .collect()
}

/// Up to `limit` bytes of a staged file, read on a blocking worker; none
/// when it cannot be opened or read.
async fn read_staged(session: &Session, staged: &StagedAttachment, limit: u64) -> Option<Vec<u8>> {
    let file = session.outgoing().open_staged(staged).await.ok()?;
    tokio::task::spawn_blocking(move || {
        let mut bytes = Vec::new();
        file.take(limit).read_to_end(&mut bytes).ok().map(|_| bytes)
    })
    .await
    .ok()
    .flatten()
}
