//! The waveform as a slider (`docs/design.md` §14.5, `audio`'s): a press or a drag seeks, the
//! pointer over it draws a hairline with the time there, ←/→ step 5 s and Space plays or
//! pauses.

use rune::{El, Key, Tx, el, text};

use super::{AudioPress, waveform_pt};
use crate::audio_store::{AudioKey, AudioShown};
use crate::design::{
    AUDIO_BAR_MAX_PT, AUDIO_FILE_PT, AUDIO_HAIRLINE_PT, AUDIO_HOVER_TIME_GAP_PT,
    AUDIO_SEEK_STEP_MS, FOCUS_RING_PT, Look, RADIUS_PILL, SPACE_SM, Surfaces, TYPE_CAPTION,
};
use crate::message_media::received::{MediaAt, MediaHost, media_element_key};
use crate::strings;

/// The waveform as a slider: pressed or dragged it seeks, the pointer over it draws the
/// hairline and the time there, ←/→ step 5 s and Space plays or pauses.
pub(super) fn slider<S: MediaHost>(
    at: MediaAt,
    key: AudioKey,
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
