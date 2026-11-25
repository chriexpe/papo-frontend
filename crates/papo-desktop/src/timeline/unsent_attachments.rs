//! Your unsent message's attachments (`docs/design.md` §14.3, §14.4,
//! §14.6): pictures in the media box (one, or an album), other files as
//! cards, each with its upload's progress. A picture waiting or uploading
//! has a scrim and a ring with ✕ inside; a file card an accent bar along
//! its bottom, "12.4 of 100 MB" and ✕ at its end. ✕ cancels the whole
//! message, never one file.
//!
//! Pure: drawn from the SDK's operation and progress and the local
//! pictures (`unsent_media`); ✕ goes to the host. The pictures are
//! `unsent_pictures`'; media in sent messages, and Save on a card, are
//! D7b's.

use papo_sdk::{
    AttachmentProgress, Operation, OperationKind, OperationState, UploadProgress, Uuid,
};
use rune::{Color, El, col, el, icon, row, text};

use super::TimelineKey;
use super::unsent_pictures::{album, single};
use super::view::TimelineHost;
use crate::attaching::extension_label;
use crate::design::{
    EXTENSION_BLOCK_PT, EXTENSION_BLOCK_RADIUS, FILE_CARD_BAR_PT, FILE_CARD_BUTTON_PT,
    FILE_CARD_MAX_PT, FILE_CARD_PAD_PT, FOCUS_RING_PT, Look, PILL_OUTLINE_PT, RADIUS_CARD,
    RADIUS_CONTROL, SPACE_MD, SPACE_SM, TYPE_BODY_MEDIUM, TYPE_CAPTION, TYPE_EXTENSION,
    UPLOAD_CANCEL_ICON_PT,
};
use crate::icons::X;
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
    /// The tallest a media box may be: the smaller of the Max height and
    /// 0.55 × the timeline's height.
    pub(super) max_height_pt: f32,
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
    let mut block = col().gap(look.space(SPACE_SM));
    match media.as_slice() {
        [] => {}
        [one] => block = block.child(single::<S>(one, cancel, unsent.max_height_pt, look)),
        many => block = block.child(album::<S>(many, cancel, look)),
    }
    for item in files {
        block = block.child(file_card::<S>(item, cancel, look));
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

/// ✕: cancels the whole message's upload.
pub(super) fn cancel_button<S: TimelineHost>(
    key: TimelineKey,
    local: Uuid,
    index: usize,
    ink: Color,
    look: &Look,
) -> El {
    row()
        .key(format!("upload-cancel-{local}-{index}"))
        .size(UPLOAD_CANCEL_ICON_PT * 2.0, UPLOAD_CANCEL_ICON_PT * 2.0)
        .center()
        .radius(UPLOAD_CANCEL_ICON_PT)
        .role("button")
        .label(strings::upload_cancel())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(move |host: &mut S| host.upload_cancelled(key, local))
        .child(icon(&X, UPLOAD_CANCEL_ICON_PT).color(ink))
}

/// A file as a card (§14.6): the extension block, the name in one line
/// (its extension kept), the size; while it uploads, "12.4 of 100 MB", a
/// 2 pt accent bar along the bottom and ✕ at the end.
fn file_card<S: TimelineHost>(
    item: &Item<'_>,
    cancel: Option<(TimelineKey, Uuid)>,
    look: &Look,
) -> El {
    let (size_line, share) = match item.upload {
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
    };
    let label = match item.size_bytes {
        Some(total) => strings::media_file(item.name, &strings::size(total)),
        None => item.name.to_owned(),
    };
    let extension = el()
        .size(EXTENSION_BLOCK_PT, EXTENSION_BLOCK_PT)
        .center()
        .radius(look.radius(EXTENSION_BLOCK_RADIUS))
        .fill(look.palette.surface_code)
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .child(
            text(extension_label(item.name))
                .font(look.font(TYPE_EXTENSION))
                .color(look.palette.text_secondary)
                .a11y_hidden(),
        );
    let words = col()
        .grow(1.0)
        .min_w(0.0)
        .child(
            text(item.name)
                .font(look.font(TYPE_BODY_MEDIUM))
                .color(look.palette.text_primary)
                .truncate_middle(),
        )
        .children(size_line.map(|line| {
            text(line)
                .font(look.font(TYPE_CAPTION).mono())
                .color(look.palette.text_muted)
        }));
    let mut card = row()
        .w_full()
        .max_w(FILE_CARD_MAX_PT)
        .pl(FILE_CARD_PAD_PT.0)
        .pr(FILE_CARD_PAD_PT.1)
        .py(FILE_CARD_PAD_PT.1)
        .gap(look.space(SPACE_MD))
        .items_center()
        .radius(look.radius(RADIUS_CARD))
        .clip_rounded(look.radius(RADIUS_CARD))
        .fill(look.palette.surface_sidebar)
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .role("group")
        .label(label)
        .child(extension)
        .child(words);
    if let (Some(share), Some((key, local))) = (share, cancel) {
        card = card
            .child(
                el().size(FILE_CARD_BUTTON_PT, FILE_CARD_BUTTON_PT)
                    .center()
                    .radius(look.radius(RADIUS_CONTROL))
                    .child(cancel_button::<S>(
                        key,
                        local,
                        item.index,
                        look.palette.text_secondary,
                        look,
                    )),
            )
            .child(
                // From the left edge, along the bottom.
                el().pin(Some(0.0), None, None, Some(0.0))
                    .w_frac(share)
                    .h(FILE_CARD_BAR_PT)
                    .fill(look.palette.accent)
                    .role("progressbar")
                    .label(progress_label(item)),
            );
    }
    card
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
