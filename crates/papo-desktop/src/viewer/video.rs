//! The viewer's video (`docs/design.md` §14.8): its frames in the picture's place, and its
//! 48 pt control bar on dark glass at the bottom: play or pause, "0:12 / 1:04" in mono, the
//! seek bar, the sound with its volume slider, shown while paused and until the pointer rests
//! 2 s while it plays. A video that can't play says so, with Save when its format is the cause.
//!
//! Pure: the player is `shell::viewer_video`'s.

use rune::{Color, El, Icon, el, icon, row, text};

use super::CLEAR_SIDE_PT;
use crate::design::{
    FOCUS_RING_PT, Look, MEDIA_FAILED_ICON_PT, PILL_ICON_PT, RADIUS_PILL, SPACE_LG, SPACE_SM,
    Surfaces, TYPE_CAPTION, TYPE_CAPTION_MEDIUM, VIDEO_BAR_GAP_PT, VIDEO_BUTTON_PT,
    VIDEO_SEEK_MIN_PT, VIDEO_SEEK_STEP_MS, VIEWER_VIDEO_BAR_PT,
};
use crate::icons::{PAUSE, PLAY, SPEAKER, VIDEO, VOLUME_OFF};
use crate::message_media::video::VideoPress;
use crate::message_media::video_bar::volume_slider;
use crate::message_media::video_track;
use crate::playback::{PlaybackFailure, PlayerState, PlayerStatus};
use crate::shell::Shell;
use crate::strings;

/// What the viewer's video draws from.
pub(crate) struct VideoShown {
    pub(crate) status: PlayerStatus,
    pub(crate) muted: bool,
    pub(crate) volume: f32,
    pub(crate) volume_open: bool,
    pub(crate) bar: bool,
}

/// The control bar along the viewer's bottom.
pub(crate) fn bar(video: &VideoShown, look: &Look) -> El {
    let status = video.status;
    let playing = status.state == PlayerState::Playing;
    let (play_icon, play_label, play_press) = if playing {
        (&PAUSE, strings::video_pause(), VideoPress::Pause)
    } else {
        (&PLAY, strings::video_play(), VideoPress::Play)
    };
    let controls = row()
        .key("viewer-video-bar")
        .h(VIEWER_VIDEO_BAR_PT)
        .w_full()
        .px(look.space(SPACE_SM))
        .gap(VIDEO_BAR_GAP_PT)
        .items_center()
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .child(button(
            "viewer-video-playing",
            play_icon,
            play_label,
            play_press,
            look,
        ))
        .child(
            text(strings::video_time(status.position_ms, status.duration_ms))
                .font(look.font(TYPE_CAPTION).mono())
                .color(look.palette.text_primary)
                .px(VIDEO_BAR_GAP_PT)
                .a11y_hidden(),
        )
        .child(seek_bar(status, look))
        .child(sound(video, look));
    el().pin(
        Some(CLEAR_SIDE_PT),
        None,
        Some(CLEAR_SIDE_PT),
        Some(look.space(SPACE_LG) + look.space(SPACE_SM)),
    )
    .child(controls)
}

/// The seek bar: a press or a drag seeks (exactly once the pointer lets go), the arrows 5 s.
fn seek_bar(status: PlayerStatus, look: &Look) -> El {
    let duration_ms = status.duration_ms.unwrap_or_default();
    #[expect(clippy::cast_precision_loss, reason = "a video's milliseconds")]
    let played = if duration_ms == 0 {
        0.0
    } else {
        (status.position_ms as f32 / duration_ms as f32).clamp(0.0, 1.0)
    };
    let step_ms = i64::try_from(VIDEO_SEEK_STEP_MS).unwrap_or(i64::MAX);
    #[expect(clippy::cast_precision_loss, reason = "a video's milliseconds")]
    let (position, length) = (status.position_ms as f64, duration_ms as f64);
    el().key("viewer-video-seek")
        .grow(1.0)
        .min_w(VIDEO_SEEK_MIN_PT)
        .h(VIDEO_BUTTON_PT)
        .center_y()
        .role("slider")
        .label(strings::video_seek())
        .a11y_value(strings::video_time(status.position_ms, status.duration_ms))
        .range_value(position, 0.0, length)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_step(move |shell: &mut Shell, delta, tx| {
            shell.viewer_video_press(VideoPress::SeekBy(i64::from(delta) * step_ms), tx);
        })
        .on_drag(|shell: &mut Shell, drag, tx| {
            let share = (drag.x / drag.w.max(1.0)).clamp(0.0, 1.0);
            let done = drag.phase == rune::Phase::Up;
            shell.viewer_video_press(VideoPress::Seek { share, done }, tx);
        })
        .child(video_track(played))
}

/// The sound: its button, and while the pointer is on it the volume slider beside it.
fn sound(video: &VideoShown, look: &Look) -> El {
    let (sound_icon, sound_label) = if video.muted {
        (&VOLUME_OFF, strings::video_unmute())
    } else {
        (&SPEAKER, strings::video_mute())
    };
    let mut sound = el()
        .on_hover_change(|shell: &mut Shell, on, tx| {
            if !shell.viewer_volume_open(on) {
                tx.live_only();
            }
        })
        .child(button(
            "viewer-video-sound",
            sound_icon,
            sound_label,
            VideoPress::Sound(video.muted),
            look,
        ));
    if video.volume_open {
        sound = sound.child(volume_slider(
            "viewer-video-volume".to_owned(),
            video.volume,
            |shell: &mut Shell, to, tx| shell.viewer_video_press(VideoPress::Volume(to), tx),
            look,
        ));
    }
    sound
}

/// A bar button: `glyph` in the pills' colours.
fn button(
    key: &'static str,
    glyph: &'static Icon,
    label: &'static str,
    press: VideoPress,
    look: &Look,
) -> El {
    el().key(key)
        .size(VIDEO_BUTTON_PT, VIDEO_BUTTON_PT)
        .center()
        .radius(VIDEO_BUTTON_PT / 2.0)
        .hover_fill(look.palette.surface_hover)
        .role("button")
        .label(label)
        .help(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(move |shell: &mut Shell, tx| shell.viewer_video_press(press, tx))
        .child(icon(glyph, PILL_ICON_PT).color(look.palette.text_primary))
}

/// A video that can't play, in the room: why, and Save when its format is the cause.
pub(crate) fn failure(failure: PlaybackFailure, look: &Look) -> El {
    let (said, action) = match failure {
        PlaybackFailure::Unsupported => {
            let (said, save) = strings::video_unsupported();
            (said, Some(save))
        }
        PlaybackFailure::Broken | PlaybackFailure::Unavailable => {
            (strings::video_failed().to_owned(), None)
        }
    };
    rune::col()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .gap(look.space(SPACE_SM))
        .child(icon(&VIDEO, MEDIA_FAILED_ICON_PT).color(Color::WHITE.alpha(0.6)))
        .child(
            text(said)
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_secondary),
        )
        .children(action.map(|save| {
            text(save)
                .font(look.font(TYPE_CAPTION_MEDIUM))
                .color(look.palette.link)
                .key("viewer-video-save")
                .role("button")
                .focusable(true)
                .on_press_tx(Shell::viewer_save)
        }))
}
