//! The viewer drawn (`docs/design.md` §14.8): black at 92 % over the whole window, the picture
//! in the room left between the pills (64 pt clear at the top, 24 at the sides and below), two
//! pills at the top, always dark (Graphite's tokens on light themes, OLED's on OLED): the author
//! with the time and "2 of 5", and Zoom out, the zoom (a press fits), Zoom in, Copy image,
//! Save, Jump to message and ✕; ‹ › as round buttons at the sides; a small ring at the top right
//! when the full picture takes more than half a second, and when it fails with nothing else to
//! draw the conversation's failed look with Retry. Spoken as a dialog named for the item.
//!
//! Pure: the state is `Viewer`'s, the handlers the shell's.

use rune::{
    Align, Color, Curve, Easing, El, Icon, Image, NativeSurface, SurfaceFit, Transition, col, el,
    icon, row, text,
};

use super::video::VideoShown;
use super::{CLEAR_SIDE_PT, CLEAR_TOP_PT, Viewer};
use crate::design::{
    FOCUS_RING_PT, HIDDEN_BLUR_PT, Look, PILL_ICON_PT, RADIUS_AVATAR, RADIUS_PILL, SPACE_LG,
    SPACE_MD, SPACE_SM, Surfaces, TYPE_CAPTION, TYPE_CAPTION_MEDIUM, TYPE_SMALL, TYPE_SMALL_MEDIUM,
    VIEWER_ARROW_PT, VIEWER_AVATAR_PT, VIEWER_BACKDROP_ALPHA, VIEWER_BUTTON_PT, VIEWER_GROW_S,
    VIEWER_RING_PT,
};
use crate::icons::{CHEVRON_LEFT, CHEVRON_RIGHT, COPY, DOWNLOAD, JUMP, X, ZOOM_IN, ZOOM_OUT};
use crate::message_media::{failed, grows_with_viewer, hidden_mark};
use crate::progress_ring::ring;
use crate::shell::Shell;
use crate::strings;

/// The viewer's element key.
pub(crate) const VIEWER_KEY: &str = "viewer";

/// What the viewer draws now.
pub(crate) struct Shown<'a> {
    pub(crate) viewer: &'a Viewer,
    /// The picture to draw: the full one once decoded, else the timeline's copy.
    pub(crate) picture: Option<&'a Image>,
    /// The picture's size in pixels, for fit and zoom.
    pub(crate) natural: [f32; 2],
    /// The room the picture has, in points (zero before the first layout).
    pub(crate) room: [f32; 2],
    /// The full picture has taken more than half a second.
    pub(crate) ring: bool,
    /// The saved pill's file name, once a save ended.
    pub(crate) saved_pill: Option<&'a str>,
    /// The item is sensitive and not revealed: blurred under the `eye-off` mark until pressed.
    pub(crate) hidden: bool,
    /// The id the item shares with its box in the conversation (only the one the viewer opened
    /// on), so the viewer grows out of the box and shrinks back into it.
    pub(crate) matched: Option<String>,
    /// A video item: its frames, why it can't play, and its controls.
    pub(crate) video: Option<ViewerVideoShown<'a>>,
}

/// A video item as the viewer draws it.
pub(crate) struct ViewerVideoShown<'a> {
    pub(crate) surface: &'a NativeSurface,
    pub(crate) failure: Option<crate::playback::PlaybackFailure>,
    pub(crate) controls: VideoShown,
}

/// The viewer over the window; `look` is already the always-dark one.
pub(crate) fn view(shown: &Shown<'_>, look: &Look) -> El {
    let viewer = shown.viewer;
    let item = viewer.item();
    let name = item.map_or("", |item| item.name.as_str());
    let label = if shown.video.is_some() {
        strings::viewer_label_video(name, viewer.current + 1, viewer.items.len())
    } else {
        strings::viewer_label(name, viewer.current + 1, viewer.items.len())
    };
    // The window keeps its full opacity, so the picture growing out of its box is seen whole
    // the whole way; the black behind it fades in and out over the same 180 ms.
    let backdrop = el()
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .fill(Color::BLACK.alpha(VIEWER_BACKDROP_ALPHA))
        .transition_motion(
            Curve::tween(VIEWER_GROW_S, Easing::EASE_OUT),
            Curve::tween(VIEWER_GROW_S, Easing::EASE_OUT),
        )
        .a11y_hidden();
    let mut root = el()
        .key(VIEWER_KEY)
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .transition(Transition::identity())
        .child(backdrop)
        .role("dialog")
        .label(label)
        .focusable(true)
        .modal_trap()
        .on_key(|shell: &mut Shell, key, tx| shell.viewer_key(key, tx))
        .child(stage(shown, look))
        .child(top(shown, look));
    if let Some(video) = &shown.video {
        root = root.on_pointer_move_local(|shell: &mut Shell, _, tx| {
            if !shell.viewer_pointer_moved(tx) {
                tx.live_only();
            }
        });
        if video.failure.is_none() && video.controls.bar {
            root = root.child(super::video::bar(&video.controls, look));
        }
    }
    if viewer.current > 0 {
        root = root.child(arrow(
            &CHEVRON_LEFT,
            strings::viewer_prev(),
            -1,
            Align::Start,
            look,
        ));
    }
    if viewer.current + 1 < viewer.items.len() {
        root = root.child(arrow(
            &CHEVRON_RIGHT,
            strings::viewer_next(),
            1,
            Align::End,
            look,
        ));
    }
    if let Some(name) = shown.saved_pill {
        root = root.child(crate::timeline::saved_pill::<Shell>(name, 0.0, look));
    }
    if shown.ring {
        root = root.child(
            el().pin(None, Some(CLEAR_TOP_PT), Some(CLEAR_SIDE_PT), None)
                .a11y_hidden()
                .child(ring(
                    None,
                    VIEWER_RING_PT,
                    Color::WHITE.alpha(0.3),
                    Color::WHITE,
                )),
        );
    }
    root
}

/// The room and the picture in it: the wheel zooms, a drag pans, a double click fits or shows
/// 100 %, a press beside the picture closes.
fn stage(shown: &Shown<'_>, look: &Look) -> El {
    let placed = shown.viewer.placed(shown.natural, shown.room);
    let [room_w, room_h] = shown.room;
    let [width, height] = placed.size;
    let x = room_w / 2.0 + placed.offset[0] - width / 2.0;
    let y = room_h / 2.0 + placed.offset[1] - height / 2.0;
    let mut pixels = el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0));
    if let Some(video) = &shown.video {
        pixels = pixels.native_surface(video.surface, SurfaceFit::Fill);
    } else if let Some(image) = shown
        .picture
        .filter(|image| image.is_ready() && image.error().is_none())
    {
        pixels = pixels.pixels_fit(image.pixels(), SurfaceFit::Fill);
    }
    // The picture clips its own blur, so a hidden one keeps its edges.
    let mut picture = el()
        .absolute(x, y)
        .size(width, height)
        .clip_rounded(0.0)
        .a11y_hidden();
    if shown.hidden {
        pixels = pixels.blur(HIDDEN_BLUR_PT);
        picture = picture.on_press(Shell::viewer_reveal);
    }
    picture = picture.child(pixels);
    if let Some(id) = shown.matched.clone() {
        picture = grows_with_viewer(picture, id);
    }
    let mut stage = el()
        .key("viewer-stage")
        .pin(
            Some(CLEAR_SIDE_PT),
            Some(CLEAR_TOP_PT),
            Some(CLEAR_SIDE_PT),
            Some(CLEAR_SIDE_PT),
        )
        .clip_rounded(0.0)
        .on_layout(|shell: &mut Shell, rect| shell.viewer_room(rect))
        .on_wheel(|shell: &mut Shell, wheel, _tx| {
            shell.viewer_wheel(wheel.dy, wheel.precise, [wheel.x, wheel.y])
        })
        .on_drag(|shell: &mut Shell, drag, _tx| shell.viewer_drag(drag))
        .on_background_press_tx(|shell: &mut Shell, tx| shell.close_viewer(tx))
        .child(picture);
    if shown.hidden {
        stage = stage.child(hidden_mark());
    }
    if let Some(failure) = shown.video.as_ref().and_then(|video| video.failure) {
        return stage.child(super::video::failure(failure, look));
    }
    // Nothing to draw and the whole picture failed: say so, with Retry. With the
    // conversation's copy drawn, that copy stays.
    let drawn = shown
        .picture
        .is_some_and(|image| image.is_ready() && image.error().is_none());
    if shown.viewer.failed && !drawn {
        let retry = text(strings::media_retry())
            .font(look.font(TYPE_CAPTION_MEDIUM))
            .color(look.palette.link)
            .key("viewer-retry")
            .role("button")
            .focusable(true)
            .on_press_tx(|shell: &mut Shell, tx| shell.viewer_retry(tx));
        stage = stage.child(failed(retry, look));
    }
    stage
}

/// The two pills at the top.
fn top(shown: &Shown<'_>, look: &Look) -> El {
    row()
        .pin(
            Some(CLEAR_SIDE_PT),
            Some(look.space(SPACE_LG)),
            Some(CLEAR_SIDE_PT),
            None,
        )
        .items_center()
        .child(author_pill(shown.viewer, look))
        .child(el().grow(1.0))
        .child(actions_pill(shown, look))
}

/// The author, the time, and "2 of 5" in an album.
fn author_pill(viewer: &Viewer, look: &Look) -> El {
    let count = (viewer.items.len() > 1)
        .then(|| strings::viewer_count(viewer.current + 1, viewer.items.len()));
    row()
        .h(VIEWER_BUTTON_PT + look.space(SPACE_SM))
        .pl(look.space(SPACE_MD))
        .pr(look.space(SPACE_LG))
        .gap(look.space(SPACE_MD))
        .center_y()
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .child(
            el().size(VIEWER_AVATAR_PT, VIEWER_AVATAR_PT)
                .radius(look.radius(RADIUS_AVATAR))
                .center()
                .fill(look.palette.surface_hover)
                .a11y_hidden()
                .child(
                    text(crate::rail::initial(&viewer.author))
                        .font(look.font(TYPE_CAPTION_MEDIUM))
                        .color(look.palette.text_secondary),
                ),
        )
        .child(
            text(viewer.author.clone())
                .font(look.font(TYPE_SMALL_MEDIUM))
                .color(look.palette.text_primary),
        )
        .child(
            text(viewer.when.clone())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_secondary),
        )
        .children(count.map(|count| {
            text(count)
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_secondary)
        }))
}

/// Zoom out, the zoom (a press fits), Zoom in, Copy image, Save, Jump to message and ✕.
fn actions_pill(shown: &Shown<'_>, look: &Look) -> El {
    let viewer = shown.viewer;
    let percent = viewer.percent(shown.natural, shown.room);
    let zoom_value = text(format!("{percent} %"))
        .key("viewer-zoom")
        .font(look.font(TYPE_SMALL).mono())
        .color(look.palette.text_primary)
        .px(look.space(SPACE_SM))
        .role("button")
        .label(strings::viewer_zoom_fit())
        .help(strings::viewer_zoom_fit())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(Shell::viewer_fit);
    row()
        .h(VIEWER_BUTTON_PT + look.space(SPACE_SM))
        .px(look.space(SPACE_SM))
        .gap(look.space(SPACE_SM) / 2.0)
        .center_y()
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .child(button(
            "viewer-zoom-out",
            &ZOOM_OUT,
            strings::viewer_zoom_out(),
            look,
            |shell, _| {
                shell.viewer_zoom_step(false);
            },
        ))
        .child(zoom_value)
        .child(button(
            "viewer-zoom-in",
            &ZOOM_IN,
            strings::viewer_zoom_in(),
            look,
            |shell, _| {
                shell.viewer_zoom_step(true);
            },
        ))
        .children(shown.video.is_none().then(|| {
            button(
                "viewer-copy",
                &COPY,
                strings::viewer_copy(),
                look,
                Shell::viewer_copy,
            )
        }))
        .child(button(
            "viewer-save",
            &DOWNLOAD,
            strings::viewer_save(),
            look,
            Shell::viewer_save,
        ))
        .child(button(
            "viewer-jump",
            &JUMP,
            strings::viewer_jump(),
            look,
            Shell::viewer_jump,
        ))
        .child(button(
            "viewer-close",
            &X,
            strings::viewer_close(),
            look,
            Shell::close_viewer,
        ))
}

/// An icon button on a pill.
pub(super) fn button(
    key: &'static str,
    glyph: &'static Icon,
    label: &'static str,
    look: &Look,
    press: impl Fn(&mut Shell, &mut rune::Tx) + 'static,
) -> El {
    el().key(key)
        .size(VIEWER_BUTTON_PT, VIEWER_BUTTON_PT)
        .center()
        .radius(VIEWER_BUTTON_PT / 2.0)
        .hover_fill(look.palette.surface_hover)
        .role("button")
        .label(label)
        .help(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(press)
        .child(icon(glyph, PILL_ICON_PT).color(look.palette.text_secondary))
}

/// ‹ or › at a side, centred: a round glass button.
fn arrow(glyph: &'static Icon, label: &'static str, by: isize, side: Align, look: &Look) -> El {
    let button = el()
        .key(if by < 0 { "viewer-prev" } else { "viewer-next" })
        .size(VIEWER_ARROW_PT, VIEWER_ARROW_PT)
        .center()
        .radius(VIEWER_ARROW_PT / 2.0)
        .pill_surface(look)
        .role("button")
        .label(label)
        .help(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(move |shell: &mut Shell, tx| {
            shell.viewer_step_tx(by, tx);
        })
        .child(icon(glyph, PILL_ICON_PT).color(look.palette.text_primary));
    let (left, right) = match side {
        Align::Start => (Some(CLEAR_SIDE_PT), None),
        _ => (None, Some(CLEAR_SIDE_PT)),
    };
    col()
        .pin(left, Some(0.0), right, Some(0.0))
        .justify_center()
        .child(button)
}
