//! The shell's side of your unsent messages' attachments (`docs/design.md`
//! §14.3): what they look like from Send on (`unsent_media`), kept in step
//! with each account's outgoing queue, and ✕, which cancels a message's
//! upload.
//!
//! At Send the tray's tiles seed the message's pictures; each drawn picture
//! is then read again from staging and decoded at the media box's size. A
//! message the queue holds that this run never drew (a relaunch) is read
//! back from staging. A message is forgotten once its queue, having listed
//! it, no longer holds it. Reading, inspecting and decoding run off the UI
//! thread.

use std::collections::HashSet;

use papo_sdk::{AccountId, OperationKind, OutgoingSnapshot, StagedAttachment, Uuid};

use super::Shell;
use super::staged_files::{decoded, read_staged};
use crate::attaching::{self, HEAD_BYTES, THUMBNAIL_SOURCE_MAX_BYTES};
use crate::composer::tray::{Tile, TileKind};
use crate::timeline::TimelineKey;
use crate::unsent_media::UnsentFile;
use papo_sdk_ui::ALBUM_CELLS;
use papo_sdk_ui::MediaSize;

/// What has been asked of staging, so each picture is read once.
#[derive(Default)]
pub(crate) struct UnsentReads {
    /// (message, attachment) pairs whose box-size decode was asked for.
    sharpening: HashSet<(Uuid, usize)>,
    /// Messages being read back after a relaunch.
    restoring: HashSet<Uuid>,
}

impl Shell {
    /// Send took `tiles` as message `local`: they are drawn from now on.
    pub(super) fn unsent_sent(&mut self, local: Uuid, tiles: &[Tile]) {
        if tiles.is_empty() {
            return;
        }
        let files = tiles.iter().map(UnsentFile::from_tile).collect();
        self.unsent.insert(local, files);
    }

    /// Account `account`'s queue changed: drawn pictures now staged are
    /// sharpened, messages this run never drew are read back, and listed
    /// messages no queue holds any more are forgotten. A sent message stays
    /// drawn while its echo shows, so its row keeps its height (§9).
    pub(super) fn unsent_follow(&mut self, account: AccountId, outgoing: &OutgoingSnapshot) {
        self.keep_own_sizes(outgoing);
        for operation in &outgoing.operations {
            let OperationKind::Message(message) = operation.kind.as_ref() else {
                continue;
            };
            if message.attachments.is_empty() {
                continue;
            }
            self.unsent.listed(operation.id);
            let staged: Vec<Option<StagedAttachment>> = message
                .attachments
                .iter()
                .map(|attachment| attachment.staged().cloned())
                .collect();
            if self.unsent.contains(operation.id) {
                self.sharpen(account, operation.id, &staged);
            } else {
                self.restore_unsent(account, operation.id, staged);
            }
        }
        let queued: HashSet<Uuid> = self
            .activity
            .iter()
            .filter(|(other, _)| **other != account)
            .filter_map(|(_, activity)| activity.outgoing.as_deref())
            .chain(std::iter::once(outgoing))
            .flat_map(|snapshot| snapshot.operations.iter().map(|operation| operation.id))
            .collect();
        self.unsent.forget_gone(|local| queued.contains(&local));
        self.unsent_reads
            .sharpening
            .retain(|(local, _)| queued.contains(local));
        self.unsent_reads
            .restoring
            .retain(|local| queued.contains(local));
    }

    /// Decodes at the box's size each drawn picture of message `local` that
    /// is staged and not yet asked for: a single picture, or an album's
    /// first cells.
    fn sharpen(&mut self, account: AccountId, local: Uuid, staged: &[Option<StagedAttachment>]) {
        let Some(entry) = self.unsent.get(local) else {
            return;
        };
        // Each drawn file, and whether it is a video still without its first frame.
        let drawn: Vec<(usize, TileKind, bool)> = entry
            .files
            .iter()
            .enumerate()
            .filter(|(_, file)| file.is_media())
            .map(|(index, file)| (index, file.kind.clone(), file.video.is_none()))
            .take(ALBUM_CELLS)
            .collect();
        for (index, kind, unread) in drawn {
            let Some(Some(staged)) = staged.get(index) else {
                continue;
            };
            if !self.unsent_reads.sharpening.insert((local, index)) {
                continue;
            }
            match kind {
                TileKind::Video if unread => {
                    self.read_unsent_video(account, local, index, staged.clone());
                }
                TileKind::Video => {}
                _ => self.read_unsent_picture(account, local, index, staged.clone()),
            }
        }
    }

    /// Reads a picture from staging and has it decoded at the box's size,
    /// off the UI thread.
    fn read_unsent_picture(
        &mut self,
        account: AccountId,
        local: Uuid,
        index: usize,
        staged: StagedAttachment,
    ) {
        let (Some(sdk), Some(images)) = (self.sdk.clone(), self.images.clone()) else {
            return;
        };
        let scale = self.scale;
        // Decoded to fit the largest box it can take; it never draws larger.
        let bounds = self.media_bounds(&self.look(false));
        let size_pt = bounds.max_width_pt.max(bounds.max_height_pt);
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let Ok(session) = client.session(account) else {
                return;
            };
            let Some(bytes) = read_staged(&session, &staged, THUMBNAIL_SOURCE_MAX_BYTES).await
            else {
                return;
            };
            let image = decoded(images.load_scaled(bytes, size_pt, scale)).await;
            updates.send(move |shell| {
                // Sent and gone meanwhile: nothing draws it.
                if !shell.unsent.contains(local) {
                    return false;
                }
                shell.unsent.sharpened(local, index, image);
                true
            });
        });
    }

    /// Reads a video from staging for its first frame, off the UI thread.
    fn read_unsent_video(
        &mut self,
        account: AccountId,
        local: Uuid,
        index: usize,
        staged: StagedAttachment,
    ) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        let engine = self.videos.engine();
        let box_px = self.video_probe_box_px();
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let Ok(session) = client.session(account) else {
                return;
            };
            let Some(video) =
                super::videos::read_local_video(&session, &staged, engine, box_px).await
            else {
                return;
            };
            updates.send(move |shell| {
                if !shell.unsent.contains(local) {
                    return false;
                }
                shell.unsent.video_read(local, index, video);
                true
            });
        });
    }

    /// Reads back message `local`, which this run never drew (it was sent
    /// before a relaunch): each staged file's kind and a picture's size
    /// from the SDK's inspection, then its pictures. Its echo waits for
    /// this, so its row is first drawn whole. A file that did not survive
    /// the relaunch (it was still copying) is drawn as a card under its
    /// name.
    fn restore_unsent(
        &mut self,
        account: AccountId,
        local: Uuid,
        staged: Vec<Option<StagedAttachment>>,
    ) {
        if !self.unsent_reads.restoring.insert(local) {
            return;
        }
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        let engine = self.videos.engine();
        let video_box_px = self.video_probe_box_px();
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let session = client.session(account).ok();
            let mut files = Vec::new();
            for staged in &staged {
                let file = match (&session, staged) {
                    (Some(session), Some(staged)) => {
                        restored_file(
                            session,
                            staged,
                            std::sync::Arc::clone(&engine),
                            video_box_px,
                        )
                        .await
                    }
                    _ => UnsentFile::unknown(),
                };
                files.push(file);
            }
            updates.send(move |shell| {
                // Gone from the queue meanwhile: nothing to draw.
                if !shell.unsent_reads.restoring.remove(&local) {
                    return false;
                }
                shell.unsent.insert(local, files);
                shell.sharpen(account, local, &staged);
                true
            });
        });
    }

    /// Whether any account has a message still preparing or uploading:
    /// leaving now would cut it off.
    pub(crate) fn uploading_anywhere(&self) -> bool {
        self.activity
            .values()
            .filter_map(|activity| activity.outgoing.as_deref())
            .any(crate::quit_ask::uploading)
    }

    /// ✕ on an uploading attachment of message `local`: the SDK cancels the
    /// whole message (Failed, or Unknown when it may have arrived).
    pub(crate) fn cancel_upload(&mut self, key: TimelineKey, local: Uuid) {
        let Some(sdk) = self.sdk.clone() else {
            return;
        };
        sdk.run(move |client| async move {
            let cancelled = match client.session(key.account) {
                Ok(session) => session.outgoing().cancel_upload(local).await,
                Err(error) => Err(error.into()),
            };
            // Already sent or failed meanwhile: the row shows what it is.
            if let Err(error) = cancelled {
                log::debug!("an upload was not cancelled: {error}");
            }
        });
    }
}

/// One staged file read back: its kind from its first bytes, and for a
/// picture its size from the SDK's inspection.
async fn restored_file(
    session: &papo_sdk::Session,
    staged: &StagedAttachment,
    engine: std::sync::Arc<dyn crate::playback::Engine>,
    video_box_px: [u32; 2],
) -> UnsentFile {
    let head = read_staged(session, staged, HEAD_BYTES as u64)
        .await
        .unwrap_or_default();
    let kind = attaching::kind_of(&head, staged.size_bytes());
    // A video's first frame, length and size, read before its row is first drawn.
    let video = if kind == TileKind::Video {
        super::videos::read_local_video(session, staged, engine, video_box_px).await
    } else {
        None
    };
    let media_size = if kind == TileKind::File {
        MediaSize::Unknown
    } else if kind == TileKind::Video {
        video
            .as_ref()
            .and_then(|video| video.size_px)
            .map_or(MediaSize::Unknown, |[width, height]| {
                MediaSize::exact(width, height)
            })
    } else {
        session
            .outgoing()
            .inspect_image(staged)
            .await
            .ok()
            .map_or(MediaSize::Unknown, |info| {
                MediaSize::exact(info.width, info.height)
            })
    };
    UnsentFile {
        kind,
        size_bytes: Some(staged.size_bytes()),
        media_size,
        image: None,
        sharp: None,
        video,
    }
}
