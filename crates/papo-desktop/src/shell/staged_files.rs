//! Reading staged files back for the tray (`docs/design.md` §14.2): a
//! stored draft's tiles after a switch or a relaunch, what the SDK's
//! inspection says of each staged file (its size, whether it can be
//! anonymized), and each picture tile's thumbnail, decoded by Rune at tile
//! size. All of it off the UI thread.
//!
//! Owns reading through `open_staged` and `inspect_image`; the tiles are
//! the composer's.

use std::io::Read;

use papo_sdk::{
    AttachmentEdit, ImageError, ImageInfo, OutgoingAttachment, OutgoingError, Session,
    StagedAttachment,
};

use super::Shell;
use crate::attaching::{self, HEAD_BYTES, Refused, THUMBNAIL_SOURCE_MAX_BYTES};
use crate::composer::tray::{Inspection, Tile, TileId, TileKind, TileSource};
use crate::notice::Refusal;
use crate::timeline::TimelineKey;

/// The side a tile's picture is decoded to fit, in points: twice the tile,
/// so a picture up to 2 : 1 still covers the 72 pt square it is cropped to.
const TILE_DECODE_PT: f32 = 144.0;

/// A stored draft's file, read back for its tile.
pub(crate) struct Restored {
    name: String,
    staged: StagedAttachment,
    kind: TileKind,
    edit: AttachmentEdit,
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
            inspection: Inspection::Pending,
            edit: file.edit,
            typed_name: None,
        }
    }

    /// Reads `files` in `key`'s tray back from staging: what the SDK's
    /// inspection says of each, and, where `thumbnail` says, its picture
    /// decoded at tile size.
    pub(super) fn read_staged_tiles(&mut self, key: TimelineKey, files: Vec<StagedRead>) {
        let (Some(sdk), Some(images)) = (self.sdk.clone(), self.images.clone()) else {
            return;
        };
        let scale = self.scale;
        for file in files {
            let updates = sdk.updates();
            let images = std::sync::Arc::clone(&images);
            sdk.run(move |client| async move {
                let Ok(session) = client.session(key.account) else {
                    return;
                };
                let inspected = session.outgoing().inspect_image(&file.staged).await;
                let inspection = inspection_of(&inspected);
                let thumbnail = if file.thumbnail {
                    read_staged(&session, &file.staged, THUMBNAIL_SOURCE_MAX_BYTES)
                        .await
                        .map(|bytes| images.load_scaled(bytes, TILE_DECODE_PT, scale))
                } else {
                    None
                };
                updates.send_tx(move |shell, tx| {
                    shell.tile_read(key, file.id, inspection, thumbnail, tx)
                });
            });
        }
    }

    /// Tile `id` of `key`'s tray was read back: what the SDK said of it,
    /// and its thumbnail. An image the SDK cannot clean goes while
    /// Anonymize is on, with a notice: it would be refused at Send.
    pub(in crate::shell) fn tile_read(
        &mut self,
        key: TimelineKey,
        id: TileId,
        inspection: Inspection,
        thumbnail: Option<rune::Image>,
        tx: &mut rune::Tx,
    ) -> bool {
        if let Some(thumbnail) = thumbnail {
            self.composer.tile_thumbnail(key, id, thumbnail);
        }
        let Some(tile) = self.composer.tile_inspected(key, id, inspection) else {
            return false;
        };
        if tile.refused_anonymized() {
            let name = tile.name.clone();
            self.take_tile_out(key, id, tx);
            self.refuse(Refusal::Attach(Refused::CannotAnonymize { name }), tx);
        }
        true
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
            updates.send(move |shell| shell.composer.tile_thumbnail(key, id, thumbnail));
        });
    }
}

/// A staged tile to read back.
pub(super) struct StagedRead {
    pub(super) id: TileId,
    pub(super) staged: StagedAttachment,
    /// It draws a picture, and its thumbnail is still to decode.
    pub(super) thumbnail: bool,
}

/// What an inspection's outcome says of the file. A failure that is not
/// the file's (storage, the account) says nothing: the SDK decides at
/// Send.
fn inspection_of(inspected: &Result<ImageInfo, OutgoingError>) -> Inspection {
    match inspected {
        Ok(info) => Inspection::Cleanable {
            width: info.width,
            height: info.height,
            animated: info.animated,
        },
        Err(OutgoingError::Image(ImageError::NotAnImage)) => Inspection::NotAnImage,
        // Reading the file failed: that says nothing of the file.
        Err(OutgoingError::Image(ImageError::Source(_))) => Inspection::Pending,
        Err(OutgoingError::Image(_)) => Inspection::Uncleanable,
        Err(_) => Inspection::Pending,
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
            edit: *attachment.edit(),
            name: attachment.filename,
            staged,
        });
    }
    restored
}

/// The staged tiles among `tiles`, to read back.
pub(super) fn staged_reads(tiles: &[Tile]) -> Vec<StagedRead> {
    tiles.iter().filter_map(StagedRead::of).collect()
}

impl StagedRead {
    /// What `tile` still needs read; none while it copies.
    pub(super) fn of(tile: &Tile) -> Option<StagedRead> {
        tile.staged().map(|staged| StagedRead {
            id: tile.id,
            staged: staged.clone(),
            thumbnail: tile.kind != TileKind::File && tile.thumbnail.is_none(),
        })
    }
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

/// `image` once Rune has decoded it, waited for on a blocking worker: a
/// view that picks a picture by whether it is ready is rebuilt when this
/// lands, which a decode finishing on its own doesn't do.
pub(super) async fn decoded(image: rune::Image) -> rune::Image {
    let waiting = image.clone();
    // A failed decode is still an answer: the view checks `error`.
    let _ = tokio::task::spawn_blocking(move || {
        let _ = waiting.wait();
    })
    .await;
    image
}
