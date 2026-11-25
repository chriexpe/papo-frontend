//! Reading staged files back for the tray (`docs/design.md` §14.2): a
//! stored draft's tiles after a switch or a relaunch, and each picture
//! tile's thumbnail, decoded by Rune at tile size off the UI thread.
//!
//! Owns reading through `open_staged`; the tiles are the composer's.

use std::io::Read;

use papo_sdk::{OutgoingAttachment, Session, StagedAttachment};

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
            size_px: None,
        }
    }

    /// Reads `pictures` in `key`'s tray back from staging: each one's size
    /// from the SDK's inspection, and, where `thumbnail` says, its picture
    /// decoded at tile size. All of it off the UI thread.
    pub(super) fn read_pictures(&mut self, key: TimelineKey, pictures: Vec<Picture>) {
        let (Some(sdk), Some(images)) = (self.sdk.clone(), self.images.clone()) else {
            return;
        };
        let scale = self.scale;
        for picture in pictures {
            let updates = sdk.updates();
            let images = std::sync::Arc::clone(&images);
            sdk.run(move |client| async move {
                let Ok(session) = client.session(key.account) else {
                    return;
                };
                let size_px = session
                    .outgoing()
                    .inspect_image(&picture.staged)
                    .await
                    .ok()
                    .map(|info| (info.width, info.height));
                let thumbnail = if picture.thumbnail {
                    read_staged(&session, &picture.staged, THUMBNAIL_SOURCE_MAX_BYTES)
                        .await
                        .map(|bytes| images.load_scaled(bytes, TILE_DECODE_PT, scale))
                } else {
                    None
                };
                updates.send(move |shell| {
                    shell
                        .composer
                        .tile_picture(key, picture.id, thumbnail, size_px)
                });
            });
        }
    }

    /// A pasted picture's bytes, already in memory: decoded at tile size
    /// off the UI thread.
    pub(super) fn decode_pasted(&mut self, key: TimelineKey, id: TileId, png: Vec<u8>) {
        let (Some(sdk), Some(images)) = (self.sdk.clone(), self.images.clone()) else {
            return;
        };
        let scale = self.scale;
        let updates = sdk.updates();
        sdk.run(move |_client| async move {
            let thumbnail = images.load_scaled(png, TILE_DECODE_PT, scale);
            updates.send(move |shell| shell.composer.tile_picture(key, id, Some(thumbnail), None));
        });
    }
}

/// A picture tile to read back from staging.
pub(super) struct Picture {
    pub(super) id: TileId,
    pub(super) staged: StagedAttachment,
    /// Its thumbnail is still to decode.
    pub(super) thumbnail: bool,
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
pub(super) fn pictures(tiles: &[Tile]) -> Vec<Picture> {
    tiles
        .iter()
        .filter(|tile| tile.kind != TileKind::File)
        .filter_map(|tile| {
            tile.staged().map(|staged| Picture {
                id: tile.id,
                staged: staged.clone(),
                thumbnail: tile.thumbnail.is_none(),
            })
        })
        .collect()
}

/// Up to `limit` bytes of a staged file, read on a blocking worker; none
/// when it cannot be opened or read.
pub(super) async fn read_staged(
    session: &Session,
    staged: &StagedAttachment,
    limit: u64,
) -> Option<Vec<u8>> {
    let file = session.outgoing().open_staged(staged).await.ok()?;
    tokio::task::spawn_blocking(move || {
        let mut bytes = Vec::new();
        file.take(limit).read_to_end(&mut bytes).ok().map(|_| bytes)
    })
    .await
    .ok()
    .flatten()
}
