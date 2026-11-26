//! The media part of a message's row (`docs/design.md` §14.3, §14.4, §15.1): its link
//! previews, then an unsent message's attachments from the local files, a received one's from
//! the server's, in the same place under the text so a sent message keeps its row's height,
//! and what each draws from for the row's memo. The drawing is `unsent_attachments`',
//! `message_media`'s and `link_cards`'.

use papo_sdk::OperationState;
use rune::{El, Spring, col};

use super::OpenTimeline;
use super::row_parts::{Origin, RowMessage};
use super::unsent_attachments::{self, Unsent};
use super::view::{Style, TimelineHost};
use crate::audio_store::AudioKey;
use crate::composer::tray::TileKind;
use crate::design::{Look, MESSAGE_BLOCK_GAP, PREVIEW_GROW_S};
use crate::media_store::MediaKey;
use crate::message_media::received::{self, MediaAt, MediaStyle, MediaSurface, attachment_id};
use crate::saving::SaveView;

/// A message's body in its parts, which a layout may place apart (§5, §6): the text, the
/// blocks under it (its link previews, then its attachments), and what follows them (its
/// reactions, a failed deletion's line).
pub(super) struct Body {
    pub(super) text: El,
    pub(super) blocks: Vec<El>,
    pub(super) after: Vec<El>,
}

impl Body {
    /// The parts one under the other, at the parent's gap.
    pub(super) fn joined(self, look: &Look) -> El {
        if self.blocks.is_empty() && self.after.is_empty() {
            return self.text;
        }
        col()
            .gap(look.space(MESSAGE_BLOCK_GAP))
            .child(self.text)
            .children(self.blocks)
            .children(self.after)
    }
}

/// `body` with the blocks that follow it, those it has: its link previews, then its
/// attachments (an unsent message's from the local files, a received one's from the server's,
/// in the same place so a sent message keeps its row's height).
pub(super) fn with_media(body: El, blocks: [Option<El>; 2], look: &Look) -> El {
    if blocks.iter().all(Option::is_none) {
        return body;
    }
    col()
        .gap(look.space(MESSAGE_BLOCK_GAP))
        .child(body)
        .children(blocks.into_iter().flatten())
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

/// How audio `key` draws, for its row's memo; none before it is known.
fn audio_drawn(media: &MediaStyle, key: AudioKey) -> Option<AudioDrawn> {
    let key = media.audio.resolve(key);
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
            let points = (share * crate::message_media::audio::waveform_pt()).round() as u32;
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
}

/// What one link preview's picture or clip draws from, for its row's memo, as an attachment's.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PreviewState(AttachmentState, Option<PlayerState>);

/// What a provider player's box draws from (`link_cards::player`): whether players load, its
/// phase, its ring, its glass buttons, its cursor, whether the viewer has it, and whether it
/// has a surface.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PlayerState {
    loadable: bool,
    shown: Option<PlayerLook>,
}

/// A player's phase, ring, glass buttons, cursor, viewer and surface, as its box draws them.
type PlayerLook = (
    std::mem::Discriminant<crate::provider_players::Phase>,
    bool,
    bool,
    std::mem::Discriminant<rune::Cursor>,
    bool,
    bool,
);

/// What a received message's attachments and link previews draw from, for the row's memo: the
/// message itself (its address: the SDK replaces a message whole when its embeds change, so it
/// stands for what its cards show), each one's state, the bounds' sides (pictures', videos',
/// then embeds'), the system's reduced motion its GIFs and videos report, and the videos'
/// volume.
pub(crate) type ReceivedSignature = (
    usize,
    Vec<AttachmentState>,
    Vec<PreviewState>,
    [u32; 6],
    bool,
    u32,
);

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
    Some((
        at,
        received_signature(at, row_message.message, &style.media),
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

/// A message from history's link previews; none for an echo or a message without any.
pub(super) fn previews_of<S: TimelineHost>(
    received: Option<&(MediaAt, ReceivedSignature)>,
    row_message: &RowMessage<'_>,
    style: &Style,
    look: &Look,
) -> Option<El> {
    let (at, _) = received?;
    let lead_gap_pt = crate::link_cards::rule_gap_pt(look, style.layout.density);
    crate::link_cards::view::previews::<S>(
        *at,
        row_message.message,
        &style.media,
        lead_gap_pt,
        look,
    )
}

/// What a received message's attachments and previews draw from, for the row's memo (a
/// pin's too).
pub(crate) fn received_signature(
    at: MediaAt,
    message: &papo_sdk::Message,
    media: &MediaStyle,
) -> ReceivedSignature {
    let attachments = message.attachments.as_deref().unwrap_or_default();
    let states = attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| {
            let key = attachment_id(attachment).map(|id| MediaKey::attachment(at.account, id));
            let hidden = received::is_hidden(at.account, attachment, &media.revealed);
            state_of(MediaAt { index, ..at }, key, hidden, media)
        })
        .collect();
    let first = attachments.len();
    let previews = crate::link_cards::previews(message, media.previews)
        .iter()
        .enumerate()
        .map(|(place, preview)| {
            let at = MediaAt {
                index: first + place,
                ..at
            };
            let key = crate::link_cards::clip_of(at.account, preview)
                .map(|(key, _)| key)
                .or_else(|| {
                    crate::link_cards::picture_key(at.account, preview).map(|(key, _)| key)
                });
            let player = crate::link_cards::player_of(at, preview).map(|(player, _)| PlayerState {
                loadable: media.players.loadable,
                shown: media.players.shown.get(&player).map(|shown| {
                    (
                        std::mem::discriminant(&shown.phase),
                        shown.ring,
                        shown.chrome,
                        std::mem::discriminant(&shown.cursor),
                        shown.in_viewer,
                        shown.surface.is_some(),
                    )
                }),
            });
            PreviewState(state_of(at, key, false, media), player)
        })
        .collect();
    (
        std::ptr::from_ref(message) as usize,
        states,
        previews,
        [
            media.bounds.max_width_pt.to_bits(),
            media.bounds.max_height_pt.to_bits(),
            media.video_bounds.max_width_pt.to_bits(),
            media.video_bounds.max_height_pt.to_bits(),
            media.embed_bounds.max_width_pt.to_bits(),
            media.embed_bounds.max_height_pt.to_bits(),
        ],
        media.reduced_motion,
        media.video_volume.to_bits(),
    )
}

/// What media `at` (`key` in the stores) draws from.
fn state_of(
    at: MediaAt,
    key: Option<MediaKey>,
    hidden: bool,
    media: &MediaStyle,
) -> AttachmentState {
    AttachmentState {
        drawn: key
            .and_then(|key| media.view.get(key))
            .map_or(0, |drawn| drawn.version),
        animates: key.is_some_and(|key| media.view.animates(key)),
        save: key.and_then(|key| media.saves.get(&key).cloned()),
        hidden,
        in_viewer: media.viewing == Some(at),
        video: key.and_then(|key| {
            let shown = *media.videos.get(key)?;
            Some((
                shown,
                media.video_attention == Some(key),
                media.video_volume_open == Some(key),
                media.video_surfaces.contains_key(&key),
            ))
        }),
        audio: key.and_then(|key| audio_drawn(media, AudioKey::Received(key))),
    }
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
        media: &style.media,
    })
}

/// What an unsent message's attachments draw from, for the row's memo (`signature`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) struct UnsentSignature {
    state: u8,
    sent_bytes: u64,
    /// The pictures' version (`unsent_media`).
    version: u64,
    /// For each file, whether a picture shows and whether it is the sharp one.
    pictures: Vec<(bool, bool)>,
    /// Each audio's key (its presses name it) and how it draws.
    audio: Vec<(AudioKey, Option<AudioDrawn>)>,
    /// The media box's sides.
    bounds: [u32; 2],
}

/// What changes an unsent message's attachments draw: its state, how much
/// has left, the pictures' version and which have decoded, its audio's
/// players, and the media box's height, for the row's memo.
pub(super) fn signature(unsent: &Unsent<'_>) -> UnsentSignature {
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
    let audio = unsent
        .files
        .iter()
        .enumerate()
        .filter(|(_, file)| file.kind == TileKind::Audio)
        .map(|(index, _)| {
            let key = AudioKey::of_operation(unsent.key.account, unsent.operation, index);
            (key, audio_drawn(unsent.media, key))
        })
        .collect();
    UnsentSignature {
        state,
        sent_bytes: sent,
        version: unsent.version,
        pictures,
        audio,
        bounds: [
            unsent.bounds.max_width_pt.to_bits(),
            unsent.bounds.max_height_pt.to_bits(),
        ],
    }
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

/// `row` growing on `spring` when it has one.
pub(super) fn glides(row: El, spring: Option<Spring>) -> El {
    match spring {
        Some(spring) => row.fold_glide(spring),
        None => row,
    }
}

/// A late preview's growth under reduced motion: one step of Rune's growth integrator
/// (1/240 s), so the row lands on the next frame.
const REDUCED_GROW_S: f32 = 1.0 / 240.0;

/// How a row that draws link previews grows when one arrives, changes or goes (§15.1): in
/// [`PREVIEW_GROW_S`], or at once under reduced motion; none for a row without previews, which
/// grows as the timeline's rows do.
pub(super) fn previews_grow(
    received: Option<&(MediaAt, ReceivedSignature)>,
    style: &Style,
) -> Option<Spring> {
    let (_, (_, _, previews, ..)) = received?;
    if previews.is_empty() {
        return None;
    }
    let seconds = if style.media.reduced_motion {
        REDUCED_GROW_S
    } else {
        PREVIEW_GROW_S
    };
    Some(Spring::new(seconds, 0.0))
}
