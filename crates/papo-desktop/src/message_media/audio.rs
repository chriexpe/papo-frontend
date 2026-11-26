//! An audio in a message (`docs/design.md` §14.5): a capsule with a play disc, the file's name
//! over its waveform (64 bars, flat until its peaks are known, played bars in `text.primary`),
//! and its time (the length at rest, the time played once played). Over the waveform a hairline
//! follows the pointer with the time under it, and a press seeks; for the keyboard the waveform
//! is a slider (←/→ 5 s, Space plays or pauses). Audio that can't be read or played is a file
//! card instead (Christian, 2026-11-25), which `received` draws.
//!
//! An unsent audio is the same player (`player`), playing its local file, with its upload's
//! size after its name, ✕ in its top corner and the accent bar along its foot (§14.3), so its
//! row keeps its size when sent.

use papo_sdk::Attachment;
use papo_sdk_ui::AUDIO_FILE_BARS;
use rune::{El, col, el, icon, row, text};

use super::received::{
    MediaAt, MediaHost, MediaPress, MediaStyle, attachment_id, media_element_key,
};
use crate::audio_store::{AudioKey, AudioShown, Reading};
use crate::design::{
    AUDIO_BAR_GAP_PT, AUDIO_BAR_MAX_PT, AUDIO_BAR_MIN_PT, AUDIO_BAR_PT, AUDIO_CANCEL_END_PT,
    AUDIO_CANCEL_TOP_PT, AUDIO_END_PAD_PT, AUDIO_FILE_PT, AUDIO_PLAY_ICON_PT, AUDIO_PLAY_PT,
    FILE_CARD_BAR_PT, FOCUS_RING_PT, Look, PILL_OUTLINE_PT, SPACE_XS, TYPE_CAPTION,
    TYPE_SMALL_MEDIUM,
};
use crate::icons::{PAUSE, PLAY};
use crate::media_store::MediaKey;
use crate::strings;

/// What a press on an audio's player asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum AudioPress {
    Play,
    Pause,
    /// To this share of its length (0 to 1).
    Seek(f32),
    /// By this many milliseconds (←/→ on the waveform).
    SeekBy(i64),
    /// The pointer is over the waveform at this share of it, or left it.
    Hover(Option<f32>),
}

/// One audio's player, as `player` draws it.
#[derive(Clone, Copy)]
pub(crate) struct Player<'a> {
    /// Its message's row, and its place among the message's attachments.
    pub(crate) at: MediaAt,
    pub(crate) key: AudioKey,
    pub(crate) name: &'a str,
    pub(crate) shown: &'a AudioShown,
    /// It takes presses and the keyboard (not in a confirmation, which shows the message only).
    pub(crate) interactive: bool,
    /// The pointer's place over its waveform (0 to 1), while there.
    pub(crate) hover: Option<f32>,
}

/// What an unsent audio's capsule adds while it uploads (§14.3).
pub(crate) struct Uploading {
    /// "1.2 of 5 MB", after the name.
    pub(crate) size_line: String,
    /// ✕, for the capsule's top corner; none when the upload can't be cancelled.
    pub(crate) cancel: Option<El>,
    /// The upload's share (0 to 1) and its spoken name: the accent bar along the foot.
    pub(crate) bar: (f32, String),
}

/// The waveform's width: its bars and the gaps between them, in points.
pub(crate) fn waveform_pt() -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "64 bars")]
    let bars = AUDIO_FILE_BARS as f32;
    bars * AUDIO_BAR_PT + (bars - 1.0) * AUDIO_BAR_GAP_PT
}

/// Received attachment `at` as an audio player, when it is audio that can play here; none
/// when it is to be a file card (it failed, or it has no ID).
pub(crate) fn audio_card<S: MediaHost>(
    at: MediaAt,
    attachment: &Attachment,
    style: &MediaStyle,
    look: &Look,
) -> Option<El> {
    let key = AudioKey::Received(MediaKey::attachment(at.account, attachment_id(attachment)?));
    let shown = style
        .audio
        .get(key)
        .cloned()
        .unwrap_or_else(AudioShown::unread);
    if matches!(shown.reading, Reading::Failed(_)) {
        return None;
    }
    let shown_player = Player {
        at,
        key,
        name: attachment.original_file_name.as_deref().unwrap_or_default(),
        shown: &shown,
        interactive: style.interactive,
        hover: hover_of(style, key),
    };
    let mut card = player::<S>(shown_player, None, look);
    if style.interactive {
        card = card.on_secondary_press(move |host: &mut S, press, tx| {
            host.media_press(at, MediaPress::Menu(Some([press.x, press.y])), tx);
        });
    }
    Some(card)
}

/// Where the pointer is over audio `key`'s waveform (0 to 1), while it is there.
pub(crate) fn hover_of(style: &MediaStyle, key: AudioKey) -> Option<f32> {
    let key = style.audio.resolve(key);
    style
        .audio_hover
        .filter(|(hovered, _)| *hovered == key && style.interactive)
        .map(|(_, share)| share)
}

/// An audio's player (received, or unsent with `uploading`): the play disc, its name over the
/// waveform, its time. Once read it plays, and its waveform seeks.
pub(crate) fn player<S: MediaHost>(
    player: Player<'_>,
    uploading: Option<Uploading>,
    look: &Look,
) -> El {
    let Player {
        at,
        key,
        name,
        shown,
        interactive,
        hover,
    } = player;
    let live = interactive && matches!(shown.reading, Reading::Read { .. });
    let waveform = waveform(shown, look);
    let waveform = if live {
        slider::<S>(at, key, shown, waveform, hover, look)
    } else {
        waveform
    };
    let time = shown.shown_ms().map(strings::clock).unwrap_or_default();
    let disc = play_disc(shown.playing, look);
    let disc = if live {
        let press = if shown.playing {
            AudioPress::Pause
        } else {
            AudioPress::Play
        };
        disc.key(format!("{}-play", media_element_key(at)))
            .role("button")
            .label(if shown.playing {
                strings::audio_pause()
            } else {
                strings::audio_play()
            })
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(move |host: &mut S, tx| host.audio_press(at, key, press, tx))
    } else {
        disc.a11y_hidden()
    };
    capsule(
        disc,
        name,
        waveform,
        time_slot(&time, shown.duration_ms(), look),
        uploading,
        look,
    )
    .key(media_element_key(at))
    .label(strings::media_audio(name))
}

/// The capsule: `disc` at its start, `name` over `waveform`, then `time`; while `uploading`,
/// the size after the name, ✕ in the top corner and the accent bar along the foot.
fn capsule(
    disc: El,
    name: &str,
    waveform: El,
    time: El,
    uploading: Option<Uploading>,
    look: &Look,
) -> El {
    let inset = (AUDIO_FILE_PT - AUDIO_PLAY_PT) / 2.0;
    let name = text(name)
        .font(look.font(TYPE_SMALL_MEDIUM))
        .color(look.palette.text_primary)
        .truncate_middle();
    let (name_line, uploading) = match uploading {
        Some(Uploading {
            size_line,
            cancel,
            bar,
        }) => {
            let size = text(format!(" · {size_line}"))
                .font(look.font(TYPE_CAPTION).mono())
                .color(look.palette.text_muted);
            let line = row().items_center().child(name.shrink()).child(size);
            (line, Some((cancel, bar)))
        }
        None => (row().child(name), None),
    };
    let words = col()
        .gap(look.space(SPACE_XS))
        .child(name_line.max_w(waveform_pt()))
        .child(waveform);
    // As wide as what it holds, at the content column's start.
    let mut built = row()
        .w_fit()
        .h(AUDIO_FILE_PT)
        .pl(inset)
        .pr(AUDIO_END_PAD_PT)
        .gap(inset)
        .items_center()
        .radius(AUDIO_FILE_PT / 2.0)
        .fill(look.palette.surface_sidebar)
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .role("group")
        .child(disc)
        .child(words)
        .child(time);
    if let Some((cancel, (share, label))) = uploading {
        built = built.clip_rounded(AUDIO_FILE_PT / 2.0).child(
            el().pin(Some(0.0), None, None, Some(0.0))
                .w_frac(share.clamp(0.0, 1.0))
                .h(FILE_CARD_BAR_PT)
                .fill(look.palette.accent)
                .role("progressbar")
                .label(label),
        );
        if let Some(cancel) = cancel {
            built = built.child(cancel.pin(
                None,
                Some(AUDIO_CANCEL_TOP_PT),
                Some(AUDIO_CANCEL_END_PT),
                None,
            ));
        }
    }
    built
}

/// The play disc: `text.primary` with its icon in `surface.content` (§14.5: neutral, not the
/// accent, so a channel of audio isn't a column of accent dots).
fn play_disc(playing: bool, look: &Look) -> El {
    let glyph = if playing { &PAUSE } else { &PLAY };
    disc(
        icon(glyph, AUDIO_PLAY_ICON_PT).color(look.palette.surface_content),
        look,
    )
}

/// The disc with `inner` at its centre.
fn disc(inner: El, look: &Look) -> El {
    el().size(AUDIO_PLAY_PT, AUDIO_PLAY_PT)
        .center()
        .radius(AUDIO_PLAY_PT / 2.0)
        .fill(look.palette.text_primary)
        .child(inner)
}

/// The time after the waveform, in mono `text.muted`; as wide as the length's, so the capsule
/// keeps its width while the time played counts up.
fn time_slot(time: &str, duration_ms: Option<u64>, look: &Look) -> El {
    let font = look.font(TYPE_CAPTION).mono();
    let widest = duration_ms.map(strings::clock).unwrap_or_default();
    col()
        .items_end()
        .child(text(time).font(font).color(look.palette.text_muted))
        .child(text(widest).font(font).h(0.0).opacity(0.0).a11y_hidden())
}

/// The bars: each from its peak, flat while unknown; the played ones in `text.primary`.
fn waveform(shown: &AudioShown, look: &Look) -> El {
    let played = shown.played_bars(AUDIO_FILE_BARS);
    let bars = (0..AUDIO_FILE_BARS).map(|index| {
        let level = shown
            .bars
            .as_ref()
            .and_then(|bars| bars.get(index).copied().flatten())
            .unwrap_or(0.0);
        let height =
            AUDIO_BAR_MIN_PT + (AUDIO_BAR_MAX_PT - AUDIO_BAR_MIN_PT) * level.clamp(0.0, 1.0);
        let colour = if index < played {
            look.palette.text_primary
        } else {
            look.palette.text_muted
        };
        el().w(AUDIO_BAR_PT)
            .h(height)
            .radius(AUDIO_BAR_PT / 2.0)
            .fill(colour)
            .a11y_hidden()
    });
    row()
        .w(waveform_pt())
        .h(AUDIO_BAR_MAX_PT)
        .gap(AUDIO_BAR_GAP_PT)
        .items_center()
        .children(bars)
}

mod slider;

use slider::slider;
