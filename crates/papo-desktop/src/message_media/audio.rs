//! An audio in a message (`docs/design.md` §14.5): a capsule with a play disc, the file's name
//! over its waveform (64 bars, flat until its peaks are known, played bars in `text.primary`),
//! and its time (the length at rest, the time played once played). Over the waveform a hairline
//! follows the pointer with the time under it, and a press seeks; for the keyboard the waveform
//! is a slider (←/→ 5 s, Space plays or pauses). Audio that can't be read or played is a file
//! card instead (Christian, 2026-11-25), which `received` draws.
//!
//! An unsent audio draws the same capsule (`capsule`), so its row keeps its height when sent.

use papo_sdk::Attachment;
use papo_sdk_ui::AUDIO_FILE_BARS;
use rune::{El, Key, Tx, col, el, icon, row, text};

use super::received::{
    MediaAt, MediaHost, MediaPress, MediaStyle, attachment_id, media_element_key,
};
use crate::audio_store::{AudioShown, Reading};
use crate::design::{
    AUDIO_BAR_GAP_PT, AUDIO_BAR_MAX_PT, AUDIO_BAR_MIN_PT, AUDIO_BAR_PT, AUDIO_END_PAD_PT,
    AUDIO_FILE_PT, AUDIO_HAIRLINE_PT, AUDIO_HOVER_TIME_GAP_PT, AUDIO_PLAY_ICON_PT, AUDIO_PLAY_PT,
    AUDIO_SEEK_STEP_MS, FILE_CARD_BAR_PT, FOCUS_RING_PT, Look, PILL_OUTLINE_PT, RADIUS_PILL,
    SPACE_SM, SPACE_XS, Surfaces, TYPE_CAPTION, TYPE_SMALL_MEDIUM,
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
    let key = MediaKey {
        account: at.account,
        attachment: attachment_id(attachment)?,
    };
    let shown = style
        .audio
        .get(key)
        .cloned()
        .unwrap_or_else(AudioShown::unread);
    if matches!(shown.reading, Reading::Failed(_)) {
        return None;
    }
    let name = attachment.original_file_name.as_deref().unwrap_or_default();
    let interactive = style.interactive;
    let read = matches!(shown.reading, Reading::Read { .. });
    let hover = style
        .audio_hover
        .filter(|(hovered, _)| *hovered == key && interactive)
        .map(|(_, share)| share);
    let waveform = waveform(&shown, look);
    let waveform = if interactive && read {
        slider::<S>(at, key, &shown, waveform, hover, look)
    } else {
        waveform
    };
    let time = shown.shown_ms().map(strings::clock).unwrap_or_default();
    let disc = play_disc(shown.playing, look);
    let disc = if interactive && read {
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
    let mut card = capsule(
        disc,
        name,
        waveform,
        time_slot(&time, shown.duration_ms(), look),
        None,
        look,
    )
    .key(media_element_key(at))
    .label(strings::media_audio(name));
    if interactive {
        card = card.on_secondary_press(move |host: &mut S, press, tx| {
            host.media_press(at, MediaPress::Menu(Some([press.x, press.y])), tx);
        });
    }
    Some(card)
}

/// The capsule: `disc` at its start, `name` over `waveform`, then `time`, and when `bar` is
/// given (an upload's share and spoken name) the 2 pt accent bar along its foot.
pub(crate) fn capsule(
    disc: El,
    name: &str,
    waveform: El,
    time: El,
    bar: Option<(f32, String)>,
    look: &Look,
) -> El {
    let inset = (AUDIO_FILE_PT - AUDIO_PLAY_PT) / 2.0;
    let words = col()
        .gap(look.space(SPACE_XS))
        .child(
            text(name)
                .font(look.font(TYPE_SMALL_MEDIUM))
                .color(look.palette.text_primary)
                .max_w(waveform_pt())
                .truncate_middle(),
        )
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
    if let Some((share, label)) = bar {
        built = built.clip_rounded(AUDIO_FILE_PT / 2.0).child(
            el().pin(Some(0.0), None, None, Some(0.0))
                .w_frac(share.clamp(0.0, 1.0))
                .h(FILE_CARD_BAR_PT)
                .fill(look.palette.accent)
                .role("progressbar")
                .label(label),
        );
    }
    built
}

/// The play disc: `text.primary` with its icon in `surface.content` (§14.5: neutral, not the
/// accent, so a channel of audio isn't a column of accent dots).
pub(crate) fn play_disc(playing: bool, look: &Look) -> El {
    let glyph = if playing { &PAUSE } else { &PLAY };
    disc(
        icon(glyph, AUDIO_PLAY_ICON_PT).color(look.palette.surface_content),
        look,
    )
}

/// The disc with `inner` at its centre (the play icon, or an upload's ✕).
pub(crate) fn disc(inner: El, look: &Look) -> El {
    el().size(AUDIO_PLAY_PT, AUDIO_PLAY_PT)
        .center()
        .radius(AUDIO_PLAY_PT / 2.0)
        .fill(look.palette.text_primary)
        .child(inner)
}

/// The time after the waveform, in mono `text.muted`; as wide as the length's, so the capsule
/// keeps its width while the time played counts up.
pub(crate) fn time_slot(time: &str, duration_ms: Option<u64>, look: &Look) -> El {
    let font = look.font(TYPE_CAPTION).mono();
    let widest = duration_ms.map(strings::clock).unwrap_or_default();
    col()
        .items_end()
        .child(text(time).font(font).color(look.palette.text_muted))
        .child(text(widest).font(font).h(0.0).opacity(0.0).a11y_hidden())
}

/// The bars: each from its peak, flat while unknown; the played ones in `text.primary`.
pub(crate) fn waveform(shown: &AudioShown, look: &Look) -> El {
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

/// The waveform as a slider: pressed or dragged it seeks, the pointer over it draws the
/// hairline and the time there, ←/→ step 5 s and Space plays or pauses.
fn slider<S: MediaHost>(
    at: MediaAt,
    key: MediaKey,
    shown: &AudioShown,
    bars: El,
    hover: Option<f32>,
    look: &Look,
) -> El {
    let duration_ms = shown.duration_ms().unwrap_or_default();
    #[expect(clippy::cast_precision_loss, reason = "a length in ms")]
    let duration = duration_ms as f64;
    #[expect(clippy::cast_precision_loss, reason = "a position in ms")]
    let position = shown.position_ms as f64;
    let step_ms = i64::try_from(AUDIO_SEEK_STEP_MS).unwrap_or(i64::MAX);
    let playing = shown.playing;
    let width = waveform_pt();
    let mut slider = bars
        .key(format!("{}-wave", media_element_key(at)))
        .role("slider")
        .label(strings::audio_seek(shown.position_ms, shown.duration_ms()))
        .range_value(position, 0.0, duration.max(1.0))
        .range_step(f64::from(
            u32::try_from(AUDIO_SEEK_STEP_MS).unwrap_or(5_000),
        ))
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_step(move |host: &mut S, delta, tx| {
            host.audio_press(at, key, AudioPress::SeekBy(i64::from(delta) * step_ms), tx);
        })
        .on_set_value(move |host: &mut S, value, tx| {
            #[expect(clippy::cast_possible_truncation, reason = "a share of a length")]
            let share = (value / duration.max(1.0)) as f32;
            host.audio_press(at, key, AudioPress::Seek(share), tx);
        })
        .on_drag(move |host: &mut S, drag, tx| {
            let share = (drag.x / drag.w.max(1.0)).clamp(0.0, 1.0);
            let press = if drag.phase == rune::Phase::Up {
                AudioPress::Seek(share)
            } else {
                AudioPress::Hover(Some(share))
            };
            host.audio_press(at, key, press, tx);
        })
        .on_pointer_move_local(move |host: &mut S, pointer, tx| {
            let share = (pointer.x / width).clamp(0.0, 1.0);
            host.audio_press(at, key, AudioPress::Hover(Some(share)), tx);
        })
        .on_hover_change(move |host: &mut S, on, tx| {
            if on {
                tx.live_only();
            } else {
                host.audio_press(at, key, AudioPress::Hover(None), tx);
            }
        })
        .on_key(move |host: &mut S, event: &rune::KeyEvent, tx: &mut Tx| {
            let plain = !(event.mods.ctrl || event.mods.alt || event.mods.cmd || event.mods.logo);
            let space = matches!(&event.key, Key::Text(typed) if typed == " ");
            if !(plain && space) {
                return false;
            }
            let press = if playing {
                AudioPress::Pause
            } else {
                AudioPress::Play
            };
            host.audio_press(at, key, press, tx);
            true
        });
    if let Some(share) = hover {
        let x = width * share;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a share of a length in ms"
        )]
        let at_ms = (f64::from(share) * duration) as u64;
        let hairline = el()
            .pin(Some(x), Some(0.0), None, Some(0.0))
            .w(AUDIO_HAIRLINE_PT)
            .fill(look.palette.text_primary)
            .a11y_hidden();
        // Under the hairline, below the capsule's foot, over whatever follows.
        let below_pt = f32::midpoint(AUDIO_FILE_PT, AUDIO_BAR_MAX_PT) + AUDIO_HOVER_TIME_GAP_PT;
        let time = text(strings::clock(at_ms))
            .font(look.font(TYPE_CAPTION).mono())
            .color(look.palette.text_primary)
            .px(look.space(SPACE_SM))
            .radius(look.radius(RADIUS_PILL))
            .pill_surface(look)
            .pin(Some(x), Some(below_pt), None, None)
            .raise()
            .a11y_hidden();
        slider = slider.child(hairline).child(time);
    }
    slider
}
