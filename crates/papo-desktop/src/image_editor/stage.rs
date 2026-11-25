//! The cropper's stage and controls (`docs/design.md` §14.10): the picture
//! under a fixed frame (a 2 pt white edge, the outside dimmed to 55 %
//! black, the thirds while dragging, corner handles in Free), then the zoom
//! slider between a small and a large image icon, the crop shapes and
//! Rotate, and the hint.
//!
//! Pure: drawn from the editor's state; input goes to the shell
//! (`shell/editing.rs`).

use rune::{Color, Drag, El, Image, KeyEvent, Spring, SurfaceFit, col, el, icon, row, text};

use super::crop::{Corner, Rect, Shape, ZOOM_RANGE};
use super::state::{DIALOG_WIDTH_PT, Editor, Gesture, STAGE_HEIGHT_PT, Use};
use crate::design::{
    EDITOR_FRAME_EDGE_PT, EDITOR_HANDLE_PT, EDITOR_OUTSIDE_ALPHA, EDITOR_STAGE,
    EDITOR_THIRDS_ALPHA, FOCUS_RING_PT, Look, PILL_OUTLINE_PT, RADIUS_CARD, RADIUS_CONTROL,
    SPACE_LG, SPACE_SM, TYPE_CAPTION, TYPE_SMALL_MEDIUM,
};
use crate::icons::{IMAGE, ROTATE};
use crate::shell::Shell;
use crate::strings;

/// The element key of the stage.
pub(crate) const STAGE_KEY: &str = "editor-stage";
/// The element key of the zoom slider.
pub(crate) const ZOOM_KEY: &str = "editor-zoom";
/// The slider's track width, in points.
const SLIDER_WIDTH_PT: f32 = 220.0;

/// The stage with the picture under its frame.
pub(crate) fn stage(editor: &Editor, picture: Option<&Image>, look: &Look) -> Option<El> {
    let crop = editor.crop.as_ref()?;
    let geometry = crop.geometry();
    let frame = geometry.frame;
    let shown = geometry.picture;
    let turned = crop.turns % 2 == 1;
    // The picture's own box before its turn: its sides swap back.
    let (w, h) = if turned {
        (shown.h, shown.w)
    } else {
        (shown.w, shown.h)
    };
    let mut image = el()
        .absolute(shown.x + (shown.w - w) / 2.0, shown.y + (shown.h - h) / 2.0)
        .size(w, h)
        .rotate(f32::from(crop.turns) * std::f32::consts::FRAC_PI_2)
        // A turn lands at once: from 3 back to 0 would spring backwards.
        .rotate_motion(Spring::NONE)
        .a11y_hidden();
    if let Some(picture) = picture.filter(|picture| picture.error().is_none()) {
        image = image.pixels_fit(picture.pixels(), SurfaceFit::Fill);
    }
    let radius = match editor.use_ {
        Use::Avatar => frame.w / 2.0,
        Use::Attachment | Use::Banner => look.radius(RADIUS_CARD),
    };
    let mut frame_el = el()
        .absolute(frame.x, frame.y)
        .size(frame.w, frame.h)
        .radius(radius)
        .border(EDITOR_FRAME_EDGE_PT, Color::WHITE)
        .a11y_hidden();
    if matches!(editor.gesture, Gesture::Panning { .. }) {
        frame_el = frame_el.children(thirds(frame));
    }
    let mut stage = el()
        .key(STAGE_KEY)
        .w(DIALOG_WIDTH_PT)
        .h(STAGE_HEIGHT_PT)
        .fill(EDITOR_STAGE)
        .clip_rounded(0.0)
        .role("application")
        .label(strings::editor_crop_area())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, -FOCUS_RING_PT.0, look.palette.focus_ring)
        .on_key(|shell: &mut Shell, key: &KeyEvent, _tx| shell.editor_stage_key(key))
        .on_drag(|shell: &mut Shell, drag: &mut Drag, _tx| shell.editor_drag(None, drag))
        .on_wheel(|shell: &mut Shell, wheel, _tx| shell.editor_wheel(wheel.dy, wheel.precise))
        // In a clip of its own: a turned picture draws into its own layer,
        // which would otherwise cover the dim outside the frame.
        .child(
            el().size(DIALOG_WIDTH_PT, STAGE_HEIGHT_PT)
                .clip_rounded(0.0)
                .child(image),
        )
        .children(outside(frame))
        .child(frame_el);
    if crop.shape == Shape::Free {
        stage = stage.children(handles(frame));
    }
    Some(stage)
}

/// The stage outside the frame, dimmed: four bands around it.
fn outside(frame: Rect) -> Vec<El> {
    let dim = Color::BLACK.alpha(EDITOR_OUTSIDE_ALPHA);
    let band = |x: f32, y: f32, w: f32, h: f32| {
        el().absolute(x, y)
            .size(w.max(0.0), h.max(0.0))
            .fill(dim)
            .a11y_hidden()
    };
    let bottom = frame.y + frame.h;
    let right = frame.x + frame.w;
    vec![
        band(0.0, 0.0, DIALOG_WIDTH_PT, frame.y),
        band(0.0, bottom, DIALOG_WIDTH_PT, STAGE_HEIGHT_PT - bottom),
        band(0.0, frame.y, frame.x, frame.h),
        band(right, frame.y, DIALOG_WIDTH_PT - right, frame.h),
    ]
}

/// The rule-of-thirds lines inside the frame while the picture moves.
fn thirds(frame: Rect) -> Vec<El> {
    let ink = Color::WHITE.alpha(EDITOR_THIRDS_ALPHA);
    let mut lines = Vec::new();
    for third in [1.0 / 3.0, 2.0 / 3.0] {
        lines.push(
            el().absolute(frame.w * third, 0.0)
                .size(1.0, frame.h)
                .fill(ink),
        );
        lines.push(
            el().absolute(0.0, frame.h * third)
                .size(frame.w, 1.0)
                .fill(ink),
        );
    }
    lines
}

/// A Free frame's four corner handles, each dragging its corner.
fn handles(frame: Rect) -> Vec<El> {
    let half = EDITOR_HANDLE_PT / 2.0;
    [
        (Corner::TopLeft, frame.x, frame.y),
        (Corner::TopRight, frame.x + frame.w, frame.y),
        (Corner::BottomLeft, frame.x, frame.y + frame.h),
        (Corner::BottomRight, frame.x + frame.w, frame.y + frame.h),
    ]
    .into_iter()
    .map(|(corner, x, y)| {
        el().absolute(x - half, y - half)
            .size(EDITOR_HANDLE_PT, EDITOR_HANDLE_PT)
            .radius(2.0)
            .border(3.0, Color::WHITE)
            .a11y_hidden()
            .on_drag(move |shell: &mut Shell, drag: &mut Drag, _tx| {
                shell.editor_drag(Some(corner), drag);
            })
    })
    .collect()
}

/// The controls under the stage: zoom, the crop shapes (attachments only)
/// and Rotate, then the hint.
pub(crate) fn controls(editor: &Editor, look: &Look) -> Option<El> {
    let crop = editor.crop.as_ref()?;
    let mut line = row()
        .w_full()
        .px(look.space(SPACE_LG) + look.space(SPACE_SM) * 2.0)
        .pt(look.space(SPACE_LG))
        .gap(look.space(SPACE_LG))
        .items_center()
        .child(
            icon(&IMAGE, 14.0)
                .color(look.palette.text_secondary)
                .a11y_hidden(),
        )
        .child(slider(crop.zoom, look))
        .child(
            icon(&IMAGE, 20.0)
                .color(look.palette.text_secondary)
                .a11y_hidden(),
        );
    if editor.use_ == Use::Attachment {
        line = line.child(shapes(crop.shape, look));
    }
    line = line.child(rotate_button(look));
    let hint = if crop.shape == Shape::Free {
        strings::editor_hint_free()
    } else {
        strings::editor_hint()
    };
    Some(
        col().w_full().gap(look.space(SPACE_SM)).child(line).child(
            text(hint)
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted)
                .px(look.space(SPACE_LG) + look.space(SPACE_SM) * 2.0)
                .wrap(),
        ),
    )
}

/// The zoom slider, 100 % to 400 %: a drag or a press sets it, the arrows
/// step it.
fn slider(zoom: f32, look: &Look) -> El {
    let share = (zoom - ZOOM_RANGE.0) / (ZOOM_RANGE.1 - ZOOM_RANGE.0);
    let thumb = 14.0;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a percentage from 100 to 400"
    )]
    let percent = (zoom * 100.0).round() as u32;
    el().key(ZOOM_KEY)
        .w(SLIDER_WIDTH_PT)
        .h(thumb)
        .role("slider")
        .label(format!("{}, {percent} %", strings::editor_zoom()))
        .range_value(
            f64::from(zoom),
            f64::from(ZOOM_RANGE.0),
            f64::from(ZOOM_RANGE.1),
        )
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_key(|shell: &mut Shell, key: &KeyEvent, _tx| shell.editor_zoom_key(key))
        .on_drag(move |shell: &mut Shell, drag: &mut Drag, _tx| {
            // The thumb's centre follows the pointer, as it is drawn.
            let share = (drag.x - thumb / 2.0) / (SLIDER_WIDTH_PT - thumb);
            shell.editor_zoom_to(share.clamp(0.0, 1.0));
        })
        .child(
            el().absolute(0.0, (thumb - 4.0) / 2.0)
                .size(SLIDER_WIDTH_PT, 4.0)
                .radius(2.0)
                .fill(look.palette.line_strong),
        )
        .child(
            el().absolute(0.0, (thumb - 4.0) / 2.0)
                .size(SLIDER_WIDTH_PT * share, 4.0)
                .radius(2.0)
                .fill(look.palette.accent),
        )
        .child(
            el().absolute(share * (SLIDER_WIDTH_PT - thumb), 0.0)
                .size(thumb, thumb)
                .radius(thumb / 2.0)
                .fill(Color::WHITE)
                .border(PILL_OUTLINE_PT, look.palette.line_pill),
        )
}

/// Original, Free, Square: a segmented choice.
fn shapes(current: Shape, look: &Look) -> El {
    let option = |shape: Shape, label: &'static str, key: &'static str| {
        let on = current == shape;
        let mut button = row()
            .key(key)
            .px(look.space(SPACE_LG) - 2.0)
            .py(look.space(SPACE_SM))
            .radius(look.radius(RADIUS_CONTROL) - 2.0)
            .role("radio")
            .checked(on)
            .label(label)
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press(move |shell: &mut Shell| shell.editor_shape(shape))
            .child(text(label).font(look.font(TYPE_SMALL_MEDIUM)).color(if on {
                look.palette.text_primary
            } else {
                look.palette.text_secondary
            }));
        if on {
            button = button
                .fill(look.palette.surface_pill)
                .border(PILL_OUTLINE_PT, look.palette.line_pill);
        }
        button
    };
    row()
        .role("radiogroup")
        .label(strings::editor_crop())
        .padding(2.0)
        .gap(2.0)
        .radius(look.radius(RADIUS_CONTROL))
        .fill(look.palette.surface_sidebar)
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .child(option(
            Shape::Original,
            strings::editor_original(),
            "editor-original",
        ))
        .child(option(Shape::Free, strings::editor_free(), "editor-free"))
        .child(option(
            Shape::Square,
            strings::editor_square(),
            "editor-square",
        ))
}

/// Rotate (R): a quarter turn clockwise.
fn rotate_button(look: &Look) -> El {
    row()
        .key("editor-rotate")
        .size(28.0, 28.0)
        .center()
        .radius(look.radius(RADIUS_CONTROL))
        .hover(look.palette.surface_hover)
        .role("button")
        .label(strings::editor_rotate())
        .help(strings::editor_rotate())
        .help_keys(["R"])
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(Shell::editor_rotate)
        .child(icon(&ROTATE, 16.0).color(look.palette.text_secondary))
}
