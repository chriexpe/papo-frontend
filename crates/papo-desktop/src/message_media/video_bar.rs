//! A video's control bar (`docs/design.md` §14.5): over its bottom on a scrim from nothing to
//! 55 % black, play or pause, "0:12 / 1:04" in mono, the seek bar, the sound (the `speaker`
//! icon, its volume slider grown up out of it while the pointer or keyboard is there), and
//! `fullscreen`, which opens the viewer. It fades in with the pointer over the video (120 ms,
//! none under reduced motion) and stays while a control has the keyboard or the person paused
//! it. Also the play mark and the dark glass the other marks share.
//!
//! Pure: what a control asks for goes to the host as a [`VideoPress`].

use rune::{Color, El, Tx, col, el, icon, row, text};

use super::received::{MediaAt, MediaHost, MediaStyle, media_element_key};
use super::video::VideoPress;
use crate::design::{
    FOCUS_RING_PT, Look, TYPE_CAPTION, VIDEO_BAR_GAP_PT, VIDEO_BAR_PAD_PT, VIDEO_BAR_PT,
    VIDEO_BUTTON_HOVER_ALPHA, VIDEO_BUTTON_PT, VIDEO_BUTTON_RADIUS_PT, VIDEO_GLASS_ALPHA,
    VIDEO_GLASS_BLUR_PT, VIDEO_GLASS_RGB, VIDEO_ICON_PT, VIDEO_PLAY_ALPHA, VIDEO_PLAY_ICON_PT,
    VIDEO_PLAY_PT, VIDEO_SCRIM_ALPHA, VIDEO_SEEK_MIN_PT, VIDEO_SEEK_STEP_MS, VIDEO_TRACK_ALPHA,
    VIDEO_TRACK_PT, VIDEO_VOLUME_GAP_PT, VIDEO_VOLUME_PAD_PT, VIDEO_VOLUME_PT, VIDEO_VOLUME_STEP,
};
use crate::icons::{FULLSCREEN, PAUSE, PLAY, SPEAKER, VOLUME_OFF};
use crate::media_store::MediaKey;
use crate::strings;
use crate::video_store::VideoShown;

/// The control bar of video `key` at `at`; `shown_always` when it shows without the pointer (a
/// control has the keyboard, or the person paused it).
pub(super) fn bar<S: MediaHost>(
    at: MediaAt,
    key: MediaKey,
    shown: &VideoShown,
    style: &MediaStyle,
    shown_always: bool,
    look: &Look,
) -> El {
    let element = media_element_key(at);
    let playing = shown.playing;
    let (play_icon, play_label, play_press) = if playing {
        (&PAUSE, strings::video_pause(), VideoPress::Pause)
    } else {
        (&PLAY, strings::video_play(), VideoPress::Play)
    };
    let play = button::<S>(
        format!("{element}-playing"),
        play_icon,
        play_label,
        move |host: &mut S, tx| host.video_press(at, key, play_press, tx),
        look,
    );
    let time = text(strings::video_time(shown.position_ms, shown.duration_ms()))
        .font(look.font(TYPE_CAPTION).mono())
        .color(Color::WHITE)
        .px(VIDEO_BAR_GAP_PT)
        .a11y_hidden();
    let controls = row()
        .w_full()
        .h(VIDEO_BUTTON_PT)
        .gap(VIDEO_BAR_GAP_PT)
        .items_center()
        .child(play)
        .child(time)
        .child(seek_bar::<S>(at, key, shown, look))
        .child(sound::<S>(at, key, shown, style, look))
        .child(button::<S>(
            format!("{element}-viewer"),
            &FULLSCREEN,
            strings::video_fullscreen(),
            move |host: &mut S, tx| host.video_press(at, key, VideoPress::Viewer, tx),
            look,
        ));
    let bar = el()
        .pin(Some(0.0), None, Some(0.0), Some(0.0))
        .h(VIDEO_BAR_PT)
        .pt(VIDEO_BAR_PAD_PT.0)
        .px(VIDEO_BAR_PAD_PT.1)
        .gradient(Color::TRANSPARENT, Color::BLACK.alpha(VIDEO_SCRIM_ALPHA))
        // A press on the bar's gaps is not a press on the picture.
        .on_press_tx(|_: &mut S, _| {})
        .child(controls);
    if shown_always {
        bar
    } else {
        bar.group_opacity(0.0, 1.0)
    }
}

/// A bar button: `glyph` white at 16 pt in a 28 pt square, white at 16 % while hovered.
fn button<S: MediaHost>(
    key: String,
    glyph: &'static rune::Icon,
    label: &str,
    press: impl Fn(&mut S, &mut Tx) + 'static,
    look: &Look,
) -> El {
    el().key(key)
        .size(VIDEO_BUTTON_PT, VIDEO_BUTTON_PT)
        .center()
        .radius(VIDEO_BUTTON_RADIUS_PT)
        .hover(Color::WHITE.alpha(VIDEO_BUTTON_HOVER_ALPHA))
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(press)
        .child(icon(glyph, VIDEO_ICON_PT).color(Color::WHITE))
}

/// The seek bar: a 4 pt track, white at 30 % with the played part white; a press or a drag
/// seeks (to the exact time once the pointer lets go), the arrows 5 s.
fn seek_bar<S: MediaHost>(at: MediaAt, key: MediaKey, shown: &VideoShown, look: &Look) -> El {
    let duration_ms = shown.duration_ms().unwrap_or_default();
    #[expect(
        clippy::cast_precision_loss,
        reason = "a video's milliseconds stay far below 2^24 s"
    )]
    let played = if duration_ms == 0 {
        0.0
    } else {
        (shown.position_ms as f32 / duration_ms as f32).clamp(0.0, 1.0)
    };
    let step_ms = i64::try_from(VIDEO_SEEK_STEP_MS).unwrap_or(i64::MAX);
    el().key(format!("{}-seek", media_element_key(at)))
        .grow(1.0)
        .min_w(VIDEO_SEEK_MIN_PT)
        .h(VIDEO_BUTTON_PT)
        .center_y()
        .role("slider")
        .label(strings::video_seek())
        .a11y_value(strings::video_time(shown.position_ms, shown.duration_ms()))
        .range_value(u64_f64(shown.position_ms), 0.0, u64_f64(duration_ms))
        .range_step(u64_f64(VIDEO_SEEK_STEP_MS))
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_step(move |host: &mut S, delta, tx| {
            host.video_press(at, key, VideoPress::SeekBy(i64::from(delta) * step_ms), tx);
        })
        .on_set_value(move |host: &mut S, value, tx| {
            #[expect(clippy::cast_possible_truncation, reason = "a share of a length")]
            let share = (value / u64_f64(duration_ms).max(1.0)) as f32;
            host.video_press(at, key, VideoPress::Seek { share, done: true }, tx);
        })
        .on_drag(move |host: &mut S, drag, tx| {
            let share = (drag.x / drag.w.max(1.0)).clamp(0.0, 1.0);
            let done = drag.phase == rune::Phase::Up;
            host.video_press(at, key, VideoPress::Seek { share, done }, tx);
        })
        .child(track(played, None))
}

/// The sound: its button (sound off shows `volume-off`), and while the pointer or keyboard is
/// on it the volume slider beside it.
fn sound<S: MediaHost>(
    at: MediaAt,
    key: MediaKey,
    shown: &VideoShown,
    style: &MediaStyle,
    look: &Look,
) -> El {
    let element = media_element_key(at);
    let muted = shown.muted;
    let (glyph, label) = if muted {
        (&VOLUME_OFF, strings::video_unmute())
    } else {
        (&SPEAKER, strings::video_mute())
    };
    let toggle = button::<S>(
        format!("{element}-sound"),
        glyph,
        label,
        move |host: &mut S, tx| host.video_press(at, key, VideoPress::Sound(muted), tx),
        look,
    );
    let mut group = el()
        .on_hover_change(move |host: &mut S, on, tx| host.video_volume_open(key, on, tx))
        .on_focus_change(move |host: &mut S, on, tx| host.video_volume_open(key, on, tx))
        .child(toggle);
    if style.video_volume_open == Some(key) {
        group = group.child(volume_slider(
            format!("{element}-volume"),
            style.video_volume,
            move |host: &mut S, to, tx| host.video_press(at, key, VideoPress::Volume(to), tx),
            look,
        ));
    }
    group
}

/// The volume slider over the sound button it grew out of, upward, so the button stays where
/// it is: a vertical track on dark glass, full at the top. A drag or press sets it, the arrows
/// step it; `set` gets the volume, 0 to 1.
pub(crate) fn volume_slider<S: 'static>(
    key: String,
    volume: f32,
    set: impl Fn(&mut S, f32, &mut Tx) + Clone + 'static,
    look: &Look,
) -> El {
    let (step, value, drag) = (set.clone(), set.clone(), set);
    let slider = col()
        .key(key)
        .w(VIDEO_BUTTON_PT)
        .h(VIDEO_VOLUME_PT + 2.0 * VIDEO_VOLUME_PAD_PT)
        .py(VIDEO_VOLUME_PAD_PT)
        .items_center()
        .radius(VIDEO_BUTTON_RADIUS_PT)
        .role("slider")
        .label(strings::video_volume())
        .range_value(f64::from(volume), 0.0, 1.0)
        .range_step(f64::from(VIDEO_VOLUME_STEP))
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_step(move |host: &mut S, delta, tx| {
            #[expect(clippy::cast_precision_loss, reason = "a step count")]
            let to = volume + delta as f32 * VIDEO_VOLUME_STEP;
            step(host, to.clamp(0.0, 1.0), tx);
        })
        .on_set_value(move |host: &mut S, to, tx| {
            #[expect(clippy::cast_possible_truncation, reason = "0 to 1")]
            let to = to.clamp(0.0, 1.0) as f32;
            value(host, to, tx);
        })
        .on_drag(move |host: &mut S, drag_at: &mut rune::Drag, tx| {
            // Full at the top of the track, nothing at its foot.
            let along = (drag_at.y - VIDEO_VOLUME_PAD_PT) / VIDEO_VOLUME_PT;
            drag(host, (1.0 - along).clamp(0.0, 1.0), tx);
        })
        .child(
            el().w(VIDEO_TRACK_PT)
                .h_full()
                .radius(VIDEO_TRACK_PT / 2.0)
                .fill(Color::WHITE.alpha(VIDEO_TRACK_ALPHA))
                .a11y_hidden()
                .child(
                    el().pin(Some(0.0), None, Some(0.0), Some(0.0))
                        .h_frac(volume.clamp(0.0, 1.0))
                        .radius(VIDEO_TRACK_PT / 2.0)
                        .fill(Color::WHITE),
                ),
        );
    glass(slider).pin(
        Some(0.0),
        None,
        None,
        Some(VIDEO_BUTTON_PT + VIDEO_VOLUME_GAP_PT),
    )
}

/// A 4 pt track `share` filled, white on white at 30 %.
pub(crate) fn track(share: f32, width_pt: Option<f32>) -> El {
    let rail = el()
        .h(VIDEO_TRACK_PT)
        .radius(VIDEO_TRACK_PT / 2.0)
        .fill(Color::WHITE.alpha(VIDEO_TRACK_ALPHA))
        .a11y_hidden()
        .child(
            el().pin(Some(0.0), Some(0.0), None, Some(0.0))
                .w_frac(share.clamp(0.0, 1.0))
                .radius(VIDEO_TRACK_PT / 2.0)
                .fill(Color::WHITE),
        );
    match width_pt {
        Some(width) => rail.w(width),
        None => rail.w_full(),
    }
}

/// The play mark at a video's centre: `play` on a 48 pt disc of black at 50 %; a button keyed
/// as given when `press` is (a waiting video), a mark only in an album's cell.
pub(super) fn play_mark<S: MediaHost>(
    press: Option<(String, impl Fn(&mut S, &mut Tx) + 'static)>,
    look: &Look,
) -> El {
    let disc = el()
        .size(VIDEO_PLAY_PT, VIDEO_PLAY_PT)
        .radius(VIDEO_PLAY_PT / 2.0)
        .center()
        .fill(Color::BLACK.alpha(VIDEO_PLAY_ALPHA))
        .child(icon(&PLAY, VIDEO_PLAY_ICON_PT).color(Color::WHITE));
    let disc = match press {
        Some((key, press)) => disc
            .key(key)
            .role("button")
            .label(strings::video_play())
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(press),
        None => disc.a11y_hidden(),
    };
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .child(disc)
}

/// `mark` on dark glass: the canvas's near-black at 55 %, the picture behind blurred 8 pt.
pub(crate) fn glass(mark: El) -> El {
    let [r, g, b] = VIDEO_GLASS_RGB.map(|channel| f32::from(channel) / 255.0);
    mark.fill(Color {
        r,
        g,
        b,
        a: VIDEO_GLASS_ALPHA,
    })
    .backdrop_blur(VIDEO_GLASS_BLUR_PT)
}

/// `value` as a float for accessibility ranges.
fn u64_f64(value: u64) -> f64 {
    #[expect(clippy::cast_precision_loss, reason = "milliseconds of a video")]
    let value = value as f64;
    value
}
