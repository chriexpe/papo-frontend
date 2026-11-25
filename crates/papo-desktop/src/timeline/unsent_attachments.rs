//! Your unsent message's attachments (`docs/design.md` §14.3, §14.4,
//! §14.6): pictures in the media box (one, or an album), audio as its
//! player, other files as cards, each with its upload's progress. A picture
//! waiting or uploading has a scrim and a ring with ✕ inside; a file card an
//! accent bar along its bottom, "12.4 of 100 MB" and ✕ at its end. ✕ cancels
//! the whole message, never one file.
//!
//! Pure: drawn from the SDK's operation and progress and the local
//! pictures (`unsent_media`); ✕ goes to the host. The pictures are
//! `unsent_pictures`', the audio `unsent_audio`'s; the box, album, player
//! and card are `message_media`'s, as a received message's are, so a sent
//! message keeps its row's height.

use papo_sdk::{
    AttachmentProgress, Operation, OperationKind, OperationState, UploadProgress, Uuid,
};
use papo_sdk_ui::MediaBounds;
use rune::{Color, El, col, el, icon, row};

use super::TimelineKey;
use super::unsent_audio::audio_player;
use super::unsent_pictures::{album, single};
use super::view::TimelineHost;
use crate::composer::tray::TileKind;
use crate::design::{
    FILE_CARD_BUTTON_PT, FOCUS_RING_PT, Look, MEDIA_BLOCK_GAP, RADIUS_CONTROL,
    UPLOAD_CANCEL_ICON_PT,
};
use crate::icons::X;
use crate::message_media::received::MediaStyle;
use crate::message_media::{self, FileCard};
use crate::strings;
use crate::unsent_media::UnsentFile;

/// Where one attachment's upload stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Upload {
    /// Not started: the message prepares, waits in the queue, or this
    /// file's turn has not come.
    Waiting,
    /// Part of it has left: `share` (0 to 1) of `total_bytes`.
    Sending {
        share: f32,
        sent_bytes: u64,
        total_bytes: u64,
    },
    /// All of it has left, or the message ended: nothing to show.
    Done,
}

/// What draws the attachments of unsent message `local`.
pub(super) struct Unsent<'a> {
    pub(super) key: TimelineKey,
    pub(super) local: Uuid,
    pub(super) operation: &'a Operation,
    /// The version `files` are at (`unsent_media`).
    pub(super) version: u64,
    pub(super) files: &'a [UnsentFile],
    pub(super) progress: Option<UploadProgress>,
    /// The largest box media may take here (the Image size setting, the column, 0.55 × the
    /// timeline's height).
    pub(super) bounds: MediaBounds,
    /// What media draw with: the audio players among them.
    pub(super) media: &'a MediaStyle,
}

/// The attachments block of an unsent message; none when it has none.
pub(super) fn attachments<S: TimelineHost>(unsent: &Unsent<'_>, look: &Look) -> Option<El> {
    let OperationKind::Message(message) = unsent.operation.kind.as_ref() else {
        return None;
    };
    if unsent.files.is_empty() || unsent.files.len() != message.attachments.len() {
        return None;
    }
    let uploads = uploads(unsent);
    let cancellable = matches!(
        unsent.operation.state,
        OperationState::Preparing | OperationState::Queued | OperationState::Sending
    );
    let items: Vec<Item<'_>> = message
        .attachments
        .iter()
        .zip(unsent.files)
        .zip(uploads)
        .enumerate()
        .map(|(index, ((attachment, file), upload))| Item {
            index,
            name: &attachment.filename,
            // The SDK knows it once staged, until the message is sent.
            size_bytes: Some(attachment.size_bytes())
                .filter(|size| *size > 0)
                .or(file.size_bytes),
            file,
            upload,
        })
        .collect();
    let (media, files): (Vec<&Item<'_>>, Vec<&Item<'_>>) =
        items.iter().partition(|item| item.file.is_media());
    let cancel = cancellable.then_some((unsent.key, unsent.local));
    let mut block = col().gap(look.space(MEDIA_BLOCK_GAP));
    match media.as_slice() {
        [] => {}
        [one] => block = block.child(single::<S>(one, cancel, unsent.bounds, look)),
        many => block = block.child(album::<S>(many, cancel, unsent.bounds, look)),
    }
    for item in files {
        let card = if item.file.kind == TileKind::Audio {
            audio_player::<S>(unsent, item, cancel, look)
        } else {
            file_card::<S>(item, cancel, look)
        };
        block = block.child(card);
    }
    Some(block.key("unsent-attachments"))
}

/// One attachment with what it shows.
pub(super) struct Item<'a> {
    /// Its place in the message.
    pub(super) index: usize,
    pub(super) name: &'a str,
    /// Its size, when known.
    pub(super) size_bytes: Option<u64>,
    pub(super) file: &'a UnsentFile,
    pub(super) upload: Upload,
}

/// Each attachment's upload: its share of the message's progress while it
/// sends (the SDK streams them one after another), waiting before.
fn uploads(unsent: &Unsent<'_>) -> Vec<Upload> {
    let count = unsent.files.len();
    match unsent.operation.state {
        OperationState::Preparing | OperationState::Queued => vec![Upload::Waiting; count],
        OperationState::Sending => {
            let OperationKind::Message(message) = unsent.operation.kind.as_ref() else {
                return vec![Upload::Done; count];
            };
            match unsent.progress {
                Some(progress) => progress
                    .per_attachment(message)
                    .into_iter()
                    .map(upload_of)
                    .collect(),
                None => vec![Upload::Waiting; count],
            }
        }
        _ => vec![Upload::Done; count],
    }
}

fn upload_of(progress: AttachmentProgress) -> Upload {
    // An empty file has nothing to wait for.
    if progress.bytes_uploaded >= progress.total_bytes {
        return Upload::Done;
    }
    if progress.is_waiting() {
        return Upload::Waiting;
    }
    #[expect(clippy::cast_precision_loss, reason = "a share shown on a ring")]
    let share = progress.bytes_uploaded as f32 / progress.total_bytes as f32;
    Upload::Sending {
        share,
        sent_bytes: progress.bytes_uploaded,
        total_bytes: progress.total_bytes,
    }
}

/// ✕, its icon `icon_pt` points in a round target twice that: cancels the whole message's
/// upload.
pub(super) fn cancel_button<S: TimelineHost>(
    key: TimelineKey,
    local: Uuid,
    index: usize,
    ink: Color,
    icon_pt: f32,
    look: &Look,
) -> El {
    row()
        .key(format!("upload-cancel-{local}-{index}"))
        .size(icon_pt * 2.0, icon_pt * 2.0)
        .center()
        .radius(icon_pt)
        .role("button")
        .label(strings::upload_cancel())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(move |host: &mut S| host.upload_cancelled(key, local))
        .child(icon(&X, icon_pt).color(ink))
}

/// A file as a card (§14.6): the extension block, the name in one line
/// (its extension kept), the size; while it uploads, "12.4 of 100 MB", a
/// 2 pt accent bar along the bottom and ✕ at the end.
fn file_card<S: TimelineHost>(
    item: &Item<'_>,
    cancel: Option<(TimelineKey, Uuid)>,
    look: &Look,
) -> El {
    let (size_line, share) = upload_line(item);
    let label = match item.size_bytes {
        Some(total) => strings::media_file(item.name, &strings::size(total)),
        None => item.name.to_owned(),
    };
    let card = FileCard {
        name: item.name,
        size_line: size_line.map(|line| message_media::size_line(line, look)),
        label,
    };
    let mut ends = Vec::new();
    let mut bar = None;
    if let (Some(share), Some((key, local))) = (share, cancel) {
        ends.push(
            el().size(FILE_CARD_BUTTON_PT, FILE_CARD_BUTTON_PT)
                .center()
                .radius(look.radius(RADIUS_CONTROL))
                .child(cancel_button::<S>(
                    key,
                    local,
                    item.index,
                    look.palette.text_secondary,
                    UPLOAD_CANCEL_ICON_PT,
                    look,
                )),
        );
        bar = Some((share, progress_label(item)));
    }
    message_media::file_card(card, ends, bar, look)
}

/// An unsent file's size line and its upload's share: "12.4 of 100 MB" while it waits or
/// sends, its size once sent.
pub(super) fn upload_line(item: &Item<'_>) -> (Option<String>, Option<f32>) {
    match item.upload {
        Upload::Sending {
            share,
            sent_bytes,
            total_bytes,
        } => (
            Some(strings::upload_size_progress(sent_bytes, total_bytes)),
            Some(share),
        ),
        Upload::Waiting => (
            item.size_bytes
                .map(|total| strings::upload_size_progress(0, total)),
            Some(0.0),
        ),
        Upload::Done => (item.size_bytes.map(strings::size), None),
    }
}

/// A ring's or bar's spoken name: "Uploading praia.jpg, 34 %". Readable,
/// never announced (§14.3).
pub(super) fn progress_label(item: &Item<'_>) -> String {
    let share = match item.upload {
        Upload::Sending { share, .. } => share,
        Upload::Waiting | Upload::Done => 0.0,
    };
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a clamped percentage"
    )]
    let percent = (share.clamp(0.0, 1.0) * 100.0).round() as u32;
    strings::upload_progress(item.name, percent)
}

#[cfg(test)]
mod tests {
    //! Where each file's upload stands.
    use super::*;

    #[test]
    fn an_empty_file_is_done_at_once() {
        let empty = AttachmentProgress {
            bytes_uploaded: 0,
            total_bytes: 0,
        };
        assert_eq!(upload_of(empty), Upload::Done);
    }

    #[test]
    fn a_file_not_reached_waits_and_one_fully_sent_is_done() {
        let waiting = AttachmentProgress {
            bytes_uploaded: 0,
            total_bytes: 10,
        };
        let half = AttachmentProgress {
            bytes_uploaded: 5,
            total_bytes: 10,
        };
        let whole = AttachmentProgress {
            bytes_uploaded: 10,
            total_bytes: 10,
        };
        assert_eq!(upload_of(waiting), Upload::Waiting);
        assert_eq!(
            upload_of(half),
            Upload::Sending {
                share: 0.5,
                sent_bytes: 5,
                total_bytes: 10
            }
        );
        assert_eq!(upload_of(whole), Upload::Done);
    }
}
