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

/// What one received attachment draws from, for its row's memo.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct AttachmentState {
    /// Its pictures' version in the store.
    drawn: u64,
    animates: bool,
    save: Option<SaveView>,
    /// Sensitive and not revealed.
    hidden: bool,
    /// The viewer opened on it and shows it in its place.
    in_viewer: bool,
    /// A video: what it shows (its reading, playing, sound, time), whether its controls show,
    /// its volume slider, and whether it has a surface to draw.
    video: Option<(crate::video_store::VideoShown, bool, bool, bool)>,
    /// An audio, as drawn (`AudioDrawn`).
    audio: Option<AudioDrawn>,
}

/// What an audio's player shows, for its row's memo: what it draws changes only when one of
/// these does (its time moves by seconds, its bars by bars), not with every reading of its
/// player's clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct AudioDrawn {
    reading: crate::audio_store::Reading,
    /// Its waveform's version in the store.
    bars_version: u64,
    /// The whole second it shows.
    shown_s: Option<u64>,
    played_bars: usize,
    playing: bool,
    /// The hairline's place over the waveform, in whole points.
    hover_pt: Option<u32>,
}

/// What a received message's attachments draw from, for the row's memo: each attachment's
/// state, the bounds' sides (pictures', then videos'), the system's reduced motion its GIFs
/// and videos report, and the videos' volume.
pub(crate) type ReceivedSignature = (Vec<AttachmentState>, [u32; 4], bool, u32);

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
    Some((at, received_signature(at, attachments, &style.media)))
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

/// What a received message's attachments draw from, for the row's memo (a pin's too).
pub(crate) fn received_signature(
    at: MediaAt,
    attachments: &[papo_sdk::Attachment],
    media: &MediaStyle,
) -> ReceivedSignature {
    let states = attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| {
            let key = attachment_id(attachment).map(|attachment| MediaKey {
                account: at.account,
                attachment,
            });
            AttachmentState {
                drawn: key
                    .and_then(|key| media.view.get(key))
                    .map_or(0, |drawn| drawn.version),
                animates: key.is_some_and(|key| media.view.animates(key)),
                save: key.and_then(|key| media.saves.get(&key).cloned()),
                hidden: received::is_hidden(at.account, attachment, &media.revealed),
                in_viewer: media.viewing == Some(MediaAt { index, ..at }),
                video: key.and_then(|key| {
                    let shown = *media.videos.get(key)?;
                    Some((
                        shown,
                        media.video_attention == Some(key),
                        media.video_volume_open == Some(key),
                        media.video_surfaces.contains_key(&key),
                    ))
                }),
                audio: key.and_then(|key| {
                    let shown = media.audio.get(key)?;
                    let hover_pt = media
                        .audio_hover
                        .filter(|(hovered, _)| *hovered == key)
                        .map(|(_, share)| {
                            #[expect(
                                clippy::cast_possible_truncation,
                                clippy::cast_sign_loss,
                                reason = "a place over a 254 pt waveform"
                            )]
                            let points =
                                (share * crate::message_media::audio::waveform_pt()).round() as u32;
                            points
                        });
                    Some(AudioDrawn {
                        reading: shown.reading,
                        bars_version: shown.bars_version,
                        shown_s: shown.shown_ms().map(|ms| ms / 1000),
                        played_bars: shown.played_bars(papo_sdk_ui::AUDIO_FILE_BARS),
                        playing: shown.playing,
                        hover_pt,
                    })
                }),
            }
        })
        .collect();
    (
        states,
        [
            media.bounds.max_width_pt.to_bits(),
            media.bounds.max_height_pt.to_bits(),
            media.video_bounds.max_width_pt.to_bits(),
            media.video_bounds.max_height_pt.to_bits(),
        ],
        media.reduced_motion,
        media.video_volume.to_bits(),
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
            let ready = file.shown().is_some_and(rune::Image::is_ready) || file.video.is_some();
            (ready, file.sharp_ready())
        })
        .collect()
}
