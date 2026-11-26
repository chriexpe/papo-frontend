//! A received message's attachments (`docs/design.md` §14.4–§14.6): its pictures (JPEG, PNG,
//! WebP and GIF) and videos in the media box, one alone or an album in their order, audio as a
//! player, and every other file as a card, one per line. A video is `video`'s, an audio
//! `audio`'s.
//!
//! The box is reserved from what the attachment says of its size (the SDK's rules), or, for one
//! this device sent, from the size it read from the file, so the row is the height its unsent
//! row was. A picture shows `surface.hover` while it loads, then the server's thumbnail, then
//! the original when the thumbnail is smaller than the box; a GIF animates while
//! `media_store` says so. A press opens the viewer, a secondary press or the menu key the media
//! menu; a file card saves. A picture the server marked sensitive is a spoiler: blurred under
//! the `eye-off` mark until a press reveals it, and still while hidden. Each picture shares an
//! id with the viewer's, so the viewer grows out of it and shrinks back into it.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use papo_sdk::{AccountId, Attachment, Uuid};
use papo_sdk_ui::{
    AlbumLayout, AttachmentKind, MediaBounds, MediaFit, MediaSize, Moderation, media_box,
};
use rune::{El, NativeSurface, Tx, col, el};

use super::audio::{AudioPress, audio_card};
use super::video::{self, VideoCell, VideoPress};
use super::{album, file_card_view::file_card_of, inner_edge, single_box};
use crate::audio_store::{AudioKey, AudioView};
use crate::design::{Look, MEDIA_BLOCK_GAP};
pub(crate) use crate::media_store::attachment_id;
use crate::media_store::{MediaKey, MediaView};
use crate::saving::SaveView;
use crate::strings;
use crate::video_store::VideoView;

/// What a press on received media asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum MediaPress {
    /// Open the viewer at the attachment.
    Open,
    /// Open the media menu, beside the pointer or (from the keyboard) beside the media.
    Menu(Option<[f32; 2]>),
    /// A picture that failed: ask for it again.
    Retry,
    /// A file card's Save, or Try again after a failed save.
    Save,
    /// A file card's ✕ while it saves.
    CancelSave,
    /// A saved file's Open, through the system.
    OpenSaved,
    /// A saved file's Show in folder.
    ShowSaved,
    /// A hidden (sensitive) picture: show it.
    Reveal,
}

/// Where a message's media is drawn: the same message can show in more than one place at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum MediaSurface {
    /// The conversation.
    Timeline,
    /// The pins panel.
    Pins,
    /// A confirmation showing the message.
    Confirm,
    /// The search results.
    Search,
}

/// Which attachment of which message, in the view that draws it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MediaAt {
    pub(crate) surface: MediaSurface,
    pub(crate) account: AccountId,
    pub(crate) channel: Uuid,
    pub(crate) message: Uuid,
    /// Its place among the message's attachments.
    pub(crate) index: usize,
}

/// What received media asks of the app.
pub(crate) trait MediaHost: Sized + 'static {
    /// A press on attachment `at`.
    fn media_press(&mut self, at: MediaAt, press: MediaPress, tx: &mut Tx);
    /// GIF `key`'s box at `at` is at `rect` (window points), for whether it shows (§5);
    /// `reduced_motion` is the system's, which Animate GIFs follows unless set. Two boxes can
    /// show one GIF (the same GIF sent twice shares the server's embed).
    fn media_moved(
        &mut self,
        at: MediaAt,
        key: MediaKey,
        rect: [f32; 4],
        reduced_motion: bool,
        tx: &mut Tx,
    );
    /// The pointer or the keyboard came to (or left) a GIF: with Animate GIFs off, it plays
    /// while there.
    fn media_hovered(&mut self, key: MediaKey, on: bool, reduced_motion: bool, tx: &mut Tx);
    /// The saved pill's Show in folder.
    fn saved_pill_shown(&mut self, tx: &mut Tx);
    /// A press on video `key`'s controls (at `at`).
    fn video_press(&mut self, at: MediaAt, key: MediaKey, press: VideoPress, tx: &mut Tx);
    /// Video `key`'s box is at `rect` (window points), for whether it shows (§14.5);
    /// `reduced_motion` is the system's, which Autoplay videos follows unless set.
    fn video_moved(&mut self, key: MediaKey, rect: [f32; 4], reduced_motion: bool, tx: &mut Tx);
    /// The pointer or the keyboard came to (or left) video `key`: its control bar shows.
    fn video_attention(&mut self, key: MediaKey, on: bool, tx: &mut Tx);
    /// The pointer or the keyboard came to (or left) video `key`'s sound: its volume slider
    /// shows.
    fn video_volume_open(&mut self, key: MediaKey, on: bool, tx: &mut Tx);
    /// A press on audio `key`'s player (at `at`).
    fn audio_press(&mut self, at: MediaAt, key: AudioKey, press: AudioPress, tx: &mut Tx);
}

/// What every received attachment in one view draws with.
#[derive(Clone)]
pub(crate) struct MediaStyle {
    pub(crate) view: MediaView,
    pub(crate) bounds: MediaBounds,
    /// Sizes of attachments this device sent, read from the files.
    pub(crate) own_sizes: Arc<HashMap<Uuid, MediaSize>>,
    /// File cards' saves.
    pub(crate) saves: Arc<HashMap<MediaKey, SaveView>>,
    /// The system asks for reduced motion (Animate GIFs follows it unless set).
    pub(crate) reduced_motion: bool,
    /// Media take presses and the keyboard; off inside a confirmation, which shows the message
    /// only.
    pub(crate) interactive: bool,
    /// Where these media are drawn.
    pub(crate) surface: MediaSurface,
    /// Sensitive pictures the reader revealed; the others stay hidden.
    pub(crate) revealed: Arc<HashSet<MediaKey>>,
    /// The picture the viewer opened on, while it is open: its box stays, the picture is the
    /// viewer's.
    pub(crate) viewing: Option<MediaAt>,
    /// The largest box a video takes (Video size, §14.9).
    pub(crate) video_bounds: MediaBounds,
    /// The largest box a link card's picture or a provider's poster takes (Embed size, §15.7).
    pub(crate) embed_bounds: MediaBounds,
    /// Show link previews: messages draw their cards (§15.7).
    pub(crate) previews: bool,
    /// Videos: what plays and where each is.
    pub(crate) videos: VideoView,
    /// Each read video's frames.
    pub(crate) video_surfaces: Arc<HashMap<MediaKey, NativeSurface>>,
    /// The video the pointer or keyboard is on: its control bar shows.
    pub(crate) video_attention: Option<MediaKey>,
    /// The video whose volume slider shows.
    pub(crate) video_volume_open: Option<MediaKey>,
    /// The volume videos play at, 0 to 1 (remembered on the device).
    pub(crate) video_volume: f32,
    /// Audio: what is read and what plays.
    pub(crate) audio: AudioView,
    /// The waveform the pointer is over, and where along it (0 to 1).
    pub(crate) audio_hover: Option<(AudioKey, f32)>,
    /// Provider players: whether they can load, and each one's state.
    pub(crate) players: crate::link_cards::player::PlayersView,
}

/// The id a picture shares with the viewer's while it shows it, so the viewer grows out of the
/// picture's box and shrinks back into it.
pub(crate) fn viewer_match_id(at: MediaAt) -> String {
    format!("grow-{}", media_element_key(at))
}

/// Whether `attachment` (of account `account`) is hidden now: sensitive and not revealed.
pub(crate) fn is_hidden(
    account: AccountId,
    attachment: &Attachment,
    revealed: &HashSet<MediaKey>,
) -> bool {
    if !Moderation::of(attachment).hides() {
        return false;
    }
    attachment_id(attachment)
        .is_none_or(|attachment| !revealed.contains(&MediaKey::attachment(account, attachment)))
}

/// The key of attachment `at`'s element (a picture or a file card), unique across surfaces:
/// menus grow beside it and the keyboard comes back to it.
pub(crate) fn media_element_key(at: MediaAt) -> String {
    match at.surface {
        MediaSurface::Timeline => format!("media-{}-{}", at.message, at.index),
        MediaSurface::Pins => format!("media-pin-{}-{}", at.message, at.index),
        MediaSurface::Confirm => format!("media-confirm-{}-{}", at.message, at.index),
        MediaSurface::Search => format!("media-search-{}-{}", at.message, at.index),
    }
}

impl MediaStyle {
    /// Media within `bounds`, nothing loaded, saved or sent from this device yet.
    #[cfg(test)]
    pub(crate) fn new(bounds: MediaBounds) -> MediaStyle {
        MediaStyle {
            view: MediaView::default(),
            bounds,
            own_sizes: Arc::default(),
            saves: Arc::default(),
            reduced_motion: false,
            interactive: true,
            surface: MediaSurface::Timeline,
            revealed: Arc::default(),
            viewing: None,
            video_bounds: bounds,
            embed_bounds: bounds,
            previews: true,
            videos: VideoView::default(),
            video_surfaces: Arc::default(),
            video_attention: None,
            video_volume_open: None,
            video_volume: 1.0,
            audio: AudioView::default(),
            audio_hover: None,
            players: crate::link_cards::player::PlayersView::default(),
        }
    }

    /// What `attachment` is drawn at: its size as this device read it, else as the server
    /// says.
    pub(crate) fn size_of(&self, attachment: &Attachment) -> MediaSize {
        attachment_id(attachment)
            .and_then(|id| self.own_sizes.get(&id).copied())
            .unwrap_or_else(|| MediaSize::of(attachment))
    }
}

/// Whether `attachment` draws in the media box: a picture or a video.
pub(crate) fn is_boxed(attachment: &Attachment) -> bool {
    let kind = AttachmentKind::of(attachment);
    kind.is_picture() || kind == AttachmentKind::Video
}

/// The attachments of a message, split into boxed media (pictures and videos) and cards, each
/// with its place.
pub(crate) fn split(attachments: &[Attachment]) -> (Vec<usize>, Vec<usize>) {
    (0..attachments.len()).partition(|index| is_boxed(&attachments[*index]))
}

/// The block of message `message`'s attachments at `at` (its account and channel); none when
/// it has none.
pub(crate) fn attachments<S: MediaHost>(
    at: MediaAt,
    attachments: &[Attachment],
    style: &MediaStyle,
    look: &Look,
) -> Option<El> {
    if attachments.is_empty() {
        return None;
    }
    let (pictures, cards) = split(attachments);
    let mut block = col().gap(look.space(MEDIA_BLOCK_GAP));
    let picture_at = |index: usize| MediaAt { index, ..at };
    match pictures.as_slice() {
        [] => {}
        [one] => {
            let attachment = &attachments[*one];
            let cell = if let Some(video) = video_cell(picture_at(*one), attachment, None, style) {
                let boxed = media_box(style.size_of(attachment), style.video_bounds);
                let video = VideoCell {
                    fit: boxed.fit,
                    ..video
                };
                video::single::<S>(single_box(boxed, look), &video, style, look)
            } else {
                let boxed = media_box(style.size_of(attachment), style.bounds);
                let base = single_box(boxed, look);
                picture::<S>(
                    base,
                    picture_at(*one),
                    attachment,
                    boxed.fit,
                    0,
                    style,
                    look,
                )
            };
            block = block.child(cell.child(inner_edge(look)));
        }
        many => {
            if let Some(layout) = AlbumLayout::for_count(many.len()) {
                let boxed = layout.album_box(style.bounds);
                let last = layout.shown() - 1;
                let cells = many
                    .iter()
                    .take(layout.shown())
                    .enumerate()
                    .map(|(place, index)| {
                        let more = if place == last { layout.more() } else { 0 };
                        let attachment = &attachments[*index];
                        // "+N" covers the last cell, a video's mark included.
                        let video = video_cell(picture_at(*index), attachment, None, style)
                            .filter(|_| more == 0);
                        if let Some(video) = video {
                            return video::album_cell::<S>(el(), &video, style, look);
                        }
                        picture::<S>(
                            el(),
                            picture_at(*index),
                            attachment,
                            MediaFit::Fill,
                            more,
                            style,
                            look,
                        )
                    })
                    .collect();
                block = block.child(album(layout, boxed, cells, many.len(), look));
            }
        }
    }
    for index in cards {
        let attachment = &attachments[index];
        let audio = (AttachmentKind::of(attachment) == AttachmentKind::Audio)
            .then(|| audio_card::<S>(picture_at(index), attachment, style, look))
            .flatten();
        block = block.child(
            audio.unwrap_or_else(|| file_card_of::<S>(picture_at(index), attachment, style, look)),
        );
    }
    Some(block.key(format!("{}-block", media_element_key(at))))
}

/// Attachment `at` as a video cell, when it is a video with an ID.
fn video_cell(
    at: MediaAt,
    attachment: &Attachment,
    fit: Option<MediaFit>,
    style: &MediaStyle,
) -> Option<VideoCell> {
    if AttachmentKind::of(attachment) != AttachmentKind::Video {
        return None;
    }
    let key = MediaKey::attachment(at.account, attachment_id(attachment)?);
    Some(VideoCell {
        at,
        key,
        fit: fit.unwrap_or(MediaFit::Fill),
        hidden: is_hidden(at.account, attachment, &style.revealed),
    })
}

/// One attachment's picture (`picture_cell`'s).
fn picture<S: MediaHost>(
    base: El,
    at: MediaAt,
    attachment: &Attachment,
    fit: MediaFit,
    more: usize,
    style: &MediaStyle,
    look: &Look,
) -> El {
    let gif = AttachmentKind::of(attachment) == AttachmentKind::Gif;
    let hidden = is_hidden(at.account, attachment, &style.revealed);
    let name = attachment.original_file_name.as_deref().unwrap_or_default();
    let label = if hidden {
        strings::spoiler_hidden().to_owned()
    } else if gif {
        strings::media_gif().to_owned()
    } else {
        strings::media_image(name)
    };
    let shown = Shown {
        key: attachment_id(attachment).map(|id| MediaKey::attachment(at.account, id)),
        gif,
        hidden,
        label,
    };
    picture_cell::<S>(base, at, &shown, fit, more, style, look)
}

pub(crate) use super::picture_cell::{Shown, media_key, picture_cell};
