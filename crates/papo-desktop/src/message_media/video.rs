//! A video in a message (`docs/design.md` §14.5): its frames (the first one until it plays) in
//! the media box; at rest while it plays muted, the time left and the sound-off button; while
//! it waits, its length and a play button; on hover or focus the control bar (`video_bar`); a
//! failure with Retry, or Save when its format can't play here. In an album a video is a cell
//! with the play mark and its length, and never plays there (§14.4).
//!
//! Pure: what plays is `video_store`'s, the frames are `playback`'s on a `NativeSurface`, and
//! what a press does is the host's.

use papo_sdk_ui::MediaFit;
use rune::{Color, Curve, Easing, El, Key, SurfaceFit, Tx, col, el, icon, text};

use super::hidden_mark;
use super::received::{MediaAt, MediaHost, MediaPress, MediaStyle, media_element_key};
use super::video_bar::{bar, glass, play_mark};
use crate::design::{
    FOCUS_RING_PT, HIDDEN_BLUR_PT, Look, MEDIA_FAILED_ICON_PT, SPACE_SM, TYPE_CAPTION,
    TYPE_CAPTION_MEDIUM, VIDEO_FADE_S, VIDEO_ICON_PT, VIDEO_MARK_INSET_PT, VIDEO_MUTE_PT,
};
use crate::icons::{VIDEO, VOLUME_OFF};
use crate::media_store::MediaKey;
use crate::playback::PlaybackFailure;
use crate::strings;
use crate::video_store::{Reading, VideoShown};

/// What a press on a video's controls asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum VideoPress {
    Play,
    Pause,
    /// Sound on (`true`) or off.
    Sound(bool),
    /// To this share of its length; `done` when the pointer lets go (an exact seek).
    Seek {
        share: f32,
        done: bool,
    },
    /// Back (negative) or forward by milliseconds: the seek bar's arrows.
    SeekBy(i64),
    /// The volume slider, 0 to 1.
    Volume(f32),
    /// The control bar's full screen: the viewer.
    Viewer,
    /// A failed video: read it again.
    Retry,
}

/// Where a video's element is and what it is drawn from.
pub(crate) struct VideoCell {
    pub(crate) at: MediaAt,
    pub(crate) key: MediaKey,
    pub(crate) fit: MediaFit,
    /// Sensitive and not revealed: blurred under the `eye-off` mark.
    pub(crate) hidden: bool,
}

/// A video alone in its box (`base`), with its marks and controls.
pub(crate) fn single<S: MediaHost>(
    base: El,
    cell: &VideoCell,
    style: &MediaStyle,
    look: &Look,
) -> El {
    let shown = style.videos.get(cell.key).copied();
    let at = cell.at;
    let key = cell.key;
    let mut built = frames(base, cell, style, look)
        .key(media_element_key(at))
        .role("group")
        .label(spoken(shown.as_ref()))
        .hover_group()
        // The bar fades in and out with the pointer (Rune leaves it still under reduced
        // motion).
        .hover_motion(
            Curve::tween(VIDEO_FADE_S, Easing::EASE_OUT),
            Curve::tween(VIDEO_FADE_S, Easing::EASE_OUT),
        );
    if style.interactive {
        // A press on a hidden video reveals it; the viewer and the menu still open it on
        // purpose.
        let press = if cell.hidden {
            MediaPress::Reveal
        } else {
            MediaPress::Open
        };
        let (playing, muted) = shown.map_or((false, true), |shown| (shown.playing, shown.muted));
        let reduced = style.reduced_motion;
        built = built
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(move |host: &mut S, tx| host.media_press(at, press, tx))
            .on_secondary_press(move |host: &mut S, press, tx| {
                host.media_press(at, MediaPress::Menu(Some([press.x, press.y])), tx);
            })
            .on_key(move |host: &mut S, event, tx| {
                video_key(host, at, key, (playing, muted), event, tx)
            })
            .on_focus_change(move |host: &mut S, on, tx| host.video_attention(key, on, tx))
            .on_hover_change(move |host: &mut S, on, tx| host.video_attention(key, on, tx));
        if at.surface == super::received::MediaSurface::Timeline {
            built = built.on_layout_tx(move |host: &mut S, rect, tx| {
                host.video_moved(key, rect, reduced, tx);
            });
        }
    }
    let Some(shown) = shown else {
        return built;
    };
    if cell.hidden {
        return built.child(hidden_mark());
    }
    match shown.reading {
        Reading::Failed(failure) => built.child(failure_block::<S>(at, key, failure, look)),
        Reading::Read(_) => {
            let attention = style.video_attention == Some(key);
            let resting = shown.playing && !shown.pressed;
            let waiting = !shown.playing && !shown.pressed;
            if resting {
                built = built.child(rest_marks::<S>(at, key, &shown, attention, look));
            } else if waiting {
                built = built.child(wait_marks::<S>(at, key, &shown, look));
            }
            if !waiting {
                let paused = shown.pressed && !shown.playing;
                built = built.child(bar::<S>(at, key, &shown, style, attention || paused, look));
            }
            built
        }
        Reading::Queued | Reading::Probing => built,
    }
}

/// A video as an album's cell (`base`): its first frame, the play mark and its length. A press
/// opens the viewer on it.
pub(crate) fn album_cell<S: MediaHost>(
    base: El,
    cell: &VideoCell,
    style: &MediaStyle,
    look: &Look,
) -> El {
    let shown = style.videos.get(cell.key).copied();
    let at = cell.at;
    let mut built = frames(base, cell, style, look)
        .key(media_element_key(at))
        .role("button")
        .label(spoken(shown.as_ref()));
    if style.interactive {
        let press = if cell.hidden {
            MediaPress::Reveal
        } else {
            MediaPress::Open
        };
        built = built
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(move |host: &mut S, tx| host.media_press(at, press, tx))
            .on_secondary_press(move |host: &mut S, press, tx| {
                host.media_press(at, MediaPress::Menu(Some([press.x, press.y])), tx);
            })
            .on_key(move |host: &mut S, event, tx| {
                super::received::media_key(host, at, press, event, tx)
            });
    }
    if cell.hidden {
        return built.child(hidden_mark());
    }
    match shown.map(|shown| shown.reading) {
        Some(Reading::Read(info)) => built
            .child(play_mark(None::<(String, fn(&mut S, &mut Tx))>, look))
            .child(corner_badge(
                info.duration_ms.map(strings::clock).unwrap_or_default(),
                look,
            )),
        Some(Reading::Failed(_)) => built.child(
            col()
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .center()
                .child(icon(&VIDEO, MEDIA_FAILED_ICON_PT).color(look.palette.text_muted)),
        ),
        _ => built,
    }
}

/// `base` painted with the video's surface (its first frame, then its playing frames), on
/// `surface.hover` until a frame lands (`surface.code` behind a letterboxed one).
fn frames(base: El, cell: &VideoCell, style: &MediaStyle, look: &Look) -> El {
    let (behind, fit) = match cell.fit {
        MediaFit::Fill => (look.palette.surface_hover, SurfaceFit::Fill),
        MediaFit::Letterbox => (look.palette.surface_code, SurfaceFit::Fit),
    };
    let mut built = base.fill(behind);
    let read = style
        .videos
        .get(cell.key)
        .is_some_and(|shown| matches!(shown.reading, Reading::Read(_)));
    if let Some(surface) = style.video_surfaces.get(&cell.key).filter(|_| read) {
        let mut picture = el()
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .native_surface(surface, fit)
            .a11y_hidden();
        if cell.hidden {
            picture = el()
                .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
                .clip_rounded(0.0)
                .child(picture.blur(HIDDEN_BLUR_PT));
        }
        built = built.child(picture);
    }
    built
}

/// While it plays muted at rest: the time left at the bottom left and the sound-off button at
/// the bottom right, both gone while the control bar shows.
fn rest_marks<S: MediaHost>(
    at: MediaAt,
    key: MediaKey,
    shown: &VideoShown,
    attention: bool,
    look: &Look,
) -> El {
    // A muted autoplay loops: read at its very end it is starting again, so it shows its
    // whole length rather than "0:00".
    let left = shown
        .duration_ms()
        .map(|duration| match duration.checked_sub(shown.position_ms) {
            Some(left) if left > 0 => left,
            _ => duration,
        });
    let mute = glass(
        el().size(VIDEO_MUTE_PT, VIDEO_MUTE_PT)
            .radius(VIDEO_MUTE_PT / 2.0)
            .center()
            .key(format!("{}-sound", media_element_key(at)))
            .role("button")
            .label(strings::video_unmute())
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(move |host: &mut S, tx| {
                host.video_press(at, key, VideoPress::Sound(true), tx);
            })
            .child(icon(&VOLUME_OFF, VIDEO_ICON_PT).color(Color::WHITE)),
    )
    .pin(
        None,
        None,
        Some(VIDEO_MARK_INSET_PT),
        Some(VIDEO_MARK_INSET_PT),
    );
    let marks = el()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(corner_badge(
            left.map(strings::clock).unwrap_or_default(),
            look,
        ))
        .child(mute);
    if attention {
        marks.opacity(0.0)
    } else {
        marks.group_opacity(1.0, 0.0)
    }
}

/// While it waits to be played: its length and the play button.
fn wait_marks<S: MediaHost>(at: MediaAt, key: MediaKey, shown: &VideoShown, look: &Look) -> El {
    let play = move |host: &mut S, tx: &mut Tx| host.video_press(at, key, VideoPress::Play, tx);
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .child(play_mark(
            Some((format!("{}-play", media_element_key(at)), play)),
            look,
        ))
        .child(corner_badge(
            shown.duration_ms().map(strings::clock).unwrap_or_default(),
            look,
        ))
}

/// A badge at the bottom left, the timeline's badge style.
fn corner_badge(words: String, look: &Look) -> El {
    super::length_badge(words, look)
}

/// A video that can't play: "This video's format isn't supported." with Save, or "Couldn't
/// play this video" with Retry, in the box.
fn failure_block<S: MediaHost>(
    at: MediaAt,
    key: MediaKey,
    failure: PlaybackFailure,
    look: &Look,
) -> El {
    let (said, action) = match failure {
        PlaybackFailure::Unsupported => strings::video_unsupported(),
        PlaybackFailure::Broken | PlaybackFailure::Unavailable => (
            strings::video_failed().to_owned(),
            strings::media_retry().to_owned(),
        ),
    };
    let unsupported = failure == PlaybackFailure::Unsupported;
    let action = text(action)
        .font(look.font(TYPE_CAPTION_MEDIUM))
        .color(look.palette.link)
        .key(format!("{}-failure", media_element_key(at)))
        .role("button")
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(move |host: &mut S, tx| {
            if unsupported {
                host.media_press(at, MediaPress::Save, tx);
            } else {
                host.video_press(at, key, VideoPress::Retry, tx);
            }
        });
    col()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .gap(look.space(SPACE_SM))
        .child(icon(&VIDEO, MEDIA_FAILED_ICON_PT).color(look.palette.text_muted))
        .child(
            text(said)
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted),
        )
        .child(action)
}

/// A video spoken: "Video, 1:04, muted" (§14.5's strings).
fn spoken(shown: Option<&VideoShown>) -> String {
    let muted = shown.is_some_and(|shown| shown.playing && shown.muted);
    strings::media_video(shown.and_then(VideoShown::duration_ms), muted)
}

/// Keys on a focused video (the D7 canvas): Space or K plays or pauses, M turns the sound on
/// or off, Enter opens the viewer (as a click on the picture).
fn video_key<S: MediaHost>(
    host: &mut S,
    at: MediaAt,
    key: MediaKey,
    (playing, muted): (bool, bool),
    event: &rune::KeyEvent,
    tx: &mut Tx,
) -> bool {
    let mods = event.mods;
    if mods.ctrl || mods.cmd || mods.alt || mods.logo || mods.shift {
        return false;
    }
    match &event.key {
        Key::Enter => {
            host.media_press(at, MediaPress::Open, tx);
            true
        }
        Key::Text(typed) => match typed.as_str() {
            " " | "k" | "K" => {
                let press = if playing {
                    VideoPress::Pause
                } else {
                    VideoPress::Play
                };
                host.video_press(at, key, press, tx);
                true
            }
            "m" | "M" => {
                host.video_press(at, key, VideoPress::Sound(muted), tx);
                true
            }
            _ => false,
        },
        _ => false,
    }
}
