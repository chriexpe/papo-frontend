//! The media part of a message's row (`docs/design.md` §14.3, §14.4): an unsent message's
//! attachments from the local files, a received one's from the server's, in the same place
//! under the text so a sent message keeps its row's height, and what each draws from for the
//! row's memo. The drawing is `unsent_attachments`' and `message_media`'s.

use papo_sdk::OperationState;
use rune::{El, col};

use super::OpenTimeline;
use super::row_parts::{Origin, RowMessage};
use super::unsent_attachments::{self, Unsent};
use super::view::{Style, TimelineHost};
use crate::design::{Look, MESSAGE_BLOCK_GAP};
use crate::media_store::MediaKey;
use crate::message_media::received::{self, MediaAt, MediaStyle, MediaSurface, attachment_id};
use crate::saving::SaveView;

/// `body` with its message's attachments under it, when it has any: an unsent message's from
/// the local files, a received one's from the server's, in the same place so a sent message
/// keeps its row's height.
pub(super) fn with_media(body: El, media: Option<El>, look: &Look) -> El {
    match media {
        Some(media) => col()
            .gap(look.space(MESSAGE_BLOCK_GAP))
            .child(body)
            .child(media),
        None => body,
    }
}

/// What a received message's attachments draw from, for the row's memo: each picture's load,
/// animation and save, the bounds' sides, and the system's reduced motion its GIFs report.
pub(super) type ReceivedSignature = (Vec<(u64, bool, Option<SaveView>)>, u32, u32, bool);

/// Where a message from history draws its media, and what they draw from; none for an echo.
pub(super) fn received_of(
    open: &OpenTimeline,
    row_message: &RowMessage<'_>,
    history: bool,
    style: &Style,
) -> Option<(MediaAt, ReceivedSignature)> {
    if !history {
        return None;
    }
    let at = MediaAt {
        surface: MediaSurface::Timeline,
        account: open.key.account,
        channel: open.key.channel,
        message: row_message.id,
        index: 0,
    };
    let attachments = row_message
        .message
        .attachments
        .as_deref()
        .unwrap_or_default();
    Some((
        at,
        received_signature(open.key.account, attachments, &style.media),
    ))
}

/// A message's media: an unsent one's from the local files, a received one's from the
/// server's.
pub(super) fn media_of<S: TimelineHost>(
    unsent: Option<&Unsent<'_>>,
    received: Option<&(MediaAt, ReceivedSignature)>,
    row_message: &RowMessage<'_>,
    style: &Style,
    look: &Look,
) -> Option<El> {
    if let Some(unsent) = unsent {
        return unsent_attachments::attachments::<S>(unsent, look);
    }
    let (at, _) = received?;
    let attachments = row_message
        .message
        .attachments
        .as_deref()
        .unwrap_or_default();
    received::attachments::<S>(*at, attachments, &style.media, look)
}

/// What a received message's attachments draw from, for the row's memo.
pub(super) fn received_signature(
    account: papo_sdk::AccountId,
    attachments: &[papo_sdk::Attachment],
    media: &MediaStyle,
) -> ReceivedSignature {
    let states = attachments
        .iter()
        .map(|attachment| {
            let key = attachment_id(attachment).map(|attachment| MediaKey {
                account,
                attachment,
            });
            let drawn = key
                .and_then(|key| media.view.get(key))
                .map_or(0, |drawn| drawn.version);
            let animates = key.is_some_and(|key| media.view.animates(key));
            let save = key.and_then(|key| media.saves.get(&key).cloned());
            (drawn, animates, save)
        })
        .collect();
    (
        states,
        media.bounds.max_width_pt.to_bits(),
        media.bounds.max_height_pt.to_bits(),
        media.reduced_motion,
    )
}

/// What draws an unsent message's attachments: its operation, progress
/// and local pictures. None for a message from history or one without
/// attachments. A sent one keeps them while its echo shows.
pub(super) fn unsent_of<'a>(
    open: &'a OpenTimeline,
    row_message: &RowMessage<'_>,
    style: &'a Style,
) -> Option<Unsent<'a>> {
    let Origin::Echo(_) = row_message.origin else {
        return None;
    };
    let echo = open.echoes.get(row_message.id)?;
    let operation = echo.operation();
    let entry = style.unsent.get(operation.id)?;
    Some(Unsent {
        key: open.key,
        local: operation.id,
        operation,
        version: entry.version,
        files: &entry.files,
        progress: style.reacting.outgoing.progress_for(operation.id),
        bounds: style.media.bounds,
    })
}

/// What changes an unsent message's attachments draw: its state, how much
/// has left, the pictures' version and which have decoded, and the media
/// box's height, for the row's memo.
pub(super) fn signature(unsent: &Unsent<'_>) -> (u8, u64, u64, Vec<(bool, bool)>, u32, u32) {
    let state = match unsent.operation.state {
        OperationState::Preparing => 0,
        OperationState::Queued => 1,
        OperationState::Sending => 2,
        OperationState::Failed => 3,
        OperationState::Unknown => 4,
        _ => 5,
    };
    let sent = unsent
        .progress
        .map_or(0, |progress| progress.bytes_uploaded);
    let pictures = pictures_drawn(unsent.files);
    (
        state,
        sent,
        unsent.version,
        pictures,
        unsent.bounds.max_width_pt.to_bits(),
        unsent.bounds.max_height_pt.to_bits(),
    )
}

/// For each file, whether a picture shows and whether it is the sharp
/// one: a decode landing changes what the row draws.
pub(super) fn pictures_drawn(files: &[crate::unsent_media::UnsentFile]) -> Vec<(bool, bool)> {
    files
        .iter()
        .map(|file| {
            let ready = file.shown().is_some_and(rune::Image::is_ready);
            (ready, file.sharp_ready())
        })
        .collect()
}
