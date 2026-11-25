//! The cropper's geometry (`docs/design.md` §14.10): a fixed frame on the
//! stage, the picture moving and scaling under it and always covering it,
//! quarter turns, and the part kept, in the picture's own pixels.
//!
//! Pure: stage points in, pixels out. The stage is the dialog's width by
//! 320 pt, the frame 28 pt clear of its edges; the view draws what this
//! says, and the SDK cuts what [`Crop::rect`] gives.

use papo_sdk::CropRect;

/// The frame's clearance from the stage's edges, in points.
pub(crate) const FRAME_CLEAR_PT: f32 = 28.0;
/// The smallest a Free frame may be, in points.
const FREE_MIN_PT: f32 = 48.0;
/// How far a Free frame's corners may go towards the stage's edges.
const FREE_EDGE_PT: f32 = 8.0;
/// Zoom: from covering the frame (1) to four times that.
pub(crate) const ZOOM_RANGE: (f32, f32) = (1.0, 4.0);

/// A rectangle on the stage: left, top, width, height, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rect {
    pub(crate) x: f32,
    pub(crate) y: f32,
    pub(crate) w: f32,
    pub(crate) h: f32,
}

/// The frame's shape.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Shape {
    /// The picture's own proportions: an untouched picture goes as it was.
    Original,
    /// Square.
    Square,
    /// A rectangle the corners resize.
    Free,
    /// Fixed by the use, width over height (a profile photo 1, a banner 3).
    Fixed(f32),
}

/// A frame corner Free mode resizes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// The cropper's state for one picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Crop {
    /// The picture as displayed (orientation applied), in pixels.
    pub(crate) size_px: (u32, u32),
    /// The stage, in points.
    pub(crate) stage: (f32, f32),
    pub(crate) shape: Shape,
    /// Clockwise quarter turns, 0 to 3.
    pub(crate) turns: u8,
    /// 1 covers the frame; up to 4.
    pub(crate) zoom: f32,
    /// The picture's centre on the stage; none centres it on the frame.
    centre: Option<(f32, f32)>,
    /// The Free frame; none starts it at the Original one.
    free: Option<Rect>,
}

/// Where everything is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    pub(crate) frame: Rect,
    /// The picture's box on the stage, turned: what shows under the frame.
    pub(crate) picture: Rect,
    /// Stage points per picture pixel.
    pub(crate) scale: f32,
}

impl Crop {
    /// A picture of `size_px` on a stage of `stage` points, framed `shape`.
    pub(crate) fn new(size_px: (u32, u32), stage: (f32, f32), shape: Shape) -> Crop {
        Crop {
            // A side of nothing would divide by it.
            size_px: (size_px.0.max(1), size_px.1.max(1)),
            stage,
            shape,
            turns: 0,
            zoom: ZOOM_RANGE.0,
            centre: None,
            free: None,
        }
    }

    /// The cropper as a saved edit left it: `turns` and the part kept,
    /// `rect` (none: the whole picture). Its shape is Original or Square
    /// when the part has their proportions, else Free; `fixed` keeps a
    /// use's own shape.
    pub(crate) fn restored(
        size_px: (u32, u32),
        stage: (f32, f32),
        fixed: Option<f32>,
        turns: u8,
        rect: Option<CropRect>,
    ) -> Crop {
        let mut crop = Crop::new(size_px, stage, fixed.map_or(Shape::Original, Shape::Fixed));
        crop.turns = turns % 4;
        let Some(rect) = rect else {
            return crop;
        };
        let (rw, rh) = crop.turned_px();
        #[expect(clippy::cast_precision_loss, reason = "pixel coordinates")]
        let (x, y, w, h) = (
            rect.x as f32,
            rect.y as f32,
            rect.width as f32,
            rect.height as f32,
        );
        if fixed.is_none() {
            let aspect = w / h;
            // Within a pixel of a shape is that shape; a pixel more is Free.
            crop.shape = if (w - h * rw / rh).abs() < 1.0 {
                Shape::Original
            } else if (w - h).abs() < 1.0 {
                Shape::Square
            } else {
                crop.free = Some(crop.fitted(aspect));
                Shape::Free
            };
        }
        let frame = crop.frame();
        let base = (frame.w / rw).max(frame.h / rh);
        crop.zoom = (frame.w / w / base).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        // The scale the zoom allows: the part's centre sits on the frame's.
        let scale = base * crop.zoom;
        let (part_x, part_y) = (x + w / 2.0, y + h / 2.0);
        let left = frame.x + frame.w / 2.0 - part_x * scale;
        let top = frame.y + frame.h / 2.0 - part_y * scale;
        crop.centre = Some((left + rw * scale / 2.0, top + rh * scale / 2.0));
        crop.pan(0.0, 0.0);
        crop
    }

    /// The picture's size after the turns, in pixels.
    fn turned_px(&self) -> (f32, f32) {
        #[expect(clippy::cast_precision_loss, reason = "picture sides")]
        let (w, h) = (self.size_px.0 as f32, self.size_px.1 as f32);
        if self.turns % 2 == 1 { (h, w) } else { (w, h) }
    }

    /// The frame for a shape of `aspect`, as large as the stage allows.
    fn fitted(&self, aspect: f32) -> Rect {
        let (sw, sh) = self.stage;
        let w = (sw - 2.0 * FRAME_CLEAR_PT).min((sh - 2.0 * FRAME_CLEAR_PT) * aspect);
        let h = w / aspect;
        Rect {
            x: (sw - w) / 2.0,
            y: (sh - h) / 2.0,
            w,
            h,
        }
    }

    /// The frame on the stage.
    pub(crate) fn frame(&self) -> Rect {
        let (rw, rh) = self.turned_px();
        match self.shape {
            Shape::Original => self.fitted(rw / rh),
            Shape::Square => self.fitted(1.0),
            Shape::Fixed(aspect) => self.fitted(aspect),
            Shape::Free => self.free.unwrap_or_else(|| self.fitted(rw / rh)),
        }
    }

    /// Where the frame and the picture are, the picture clamped to cover
    /// the frame.
    pub(crate) fn geometry(&self) -> Geometry {
        let frame = self.frame();
        let (rw, rh) = self.turned_px();
        let scale = (frame.w / rw).max(frame.h / rh) * self.zoom;
        let (w, h) = (rw * scale, rh * scale);
        let (cx, cy) = self
            .centre
            .unwrap_or((frame.x + frame.w / 2.0, frame.y + frame.h / 2.0));
        let cx = within(cx, frame.x + frame.w - w / 2.0, frame.x + w / 2.0);
        let cy = within(cy, frame.y + frame.h - h / 2.0, frame.y + h / 2.0);
        Geometry {
            frame,
            picture: Rect {
                x: cx - w / 2.0,
                y: cy - h / 2.0,
                w,
                h,
            },
            scale,
        }
    }

    /// Moves the picture by (`dx`, `dy`) points; it stops where it would
    /// leave the frame uncovered.
    pub(crate) fn pan(&mut self, dx: f32, dy: f32) {
        let picture = self.geometry().picture;
        self.centre = Some((
            picture.x + picture.w / 2.0 + dx,
            picture.y + picture.h / 2.0 + dy,
        ));
        // Settle the clamp, so a pan past an edge leaves no slack behind.
        let settled = self.geometry().picture;
        self.centre = Some((settled.x + settled.w / 2.0, settled.y + settled.h / 2.0));
    }

    /// Sets the zoom, kept within [`ZOOM_RANGE`]; a zoom that is no number
    /// is ignored.
    pub(crate) fn set_zoom(&mut self, zoom: f32) {
        if !zoom.is_finite() {
            return;
        }
        let before = self.geometry();
        self.zoom = zoom.clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        self.keep_frame_centre(before);
    }

    /// After a change of scale, puts the point of the picture that was at
    /// the frame's centre in `before` back there.
    fn keep_frame_centre(&mut self, before: Geometry) {
        let after = self.geometry();
        let (fx, fy) = (
            after.frame.x + after.frame.w / 2.0,
            after.frame.y + after.frame.h / 2.0,
        );
        let (bx, by) = (
            before.frame.x + before.frame.w / 2.0,
            before.frame.y + before.frame.h / 2.0,
        );
        // Where that point sat in the picture, as a share of its sides.
        let share_x = (bx - before.picture.x) / before.picture.w;
        let share_y = (by - before.picture.y) / before.picture.h;
        let left = fx - share_x * after.picture.w;
        let top = fy - share_y * after.picture.h;
        self.centre = Some((left + after.picture.w / 2.0, top + after.picture.h / 2.0));
        self.pan(0.0, 0.0);
    }

    /// Turns the picture a quarter clockwise; the frame starts again from
    /// the shape (a Free frame from the picture's proportions).
    pub(crate) fn rotate(&mut self) {
        self.turns = (self.turns + 1) % 4;
        self.centre = None;
        self.free = None;
    }

    /// Back to the start: no zoom, centred (turns and shape stay).
    pub(crate) fn reset(&mut self) {
        self.zoom = ZOOM_RANGE.0;
        self.centre = None;
    }

    /// Changes the frame's shape; the picture centres again.
    pub(crate) fn set_shape(&mut self, shape: Shape) {
        if shape == Shape::Free && self.shape != Shape::Free {
            self.free = Some(self.frame());
        }
        self.shape = shape;
        self.centre = None;
    }

    /// A Free frame's `corner` dragged by (`dx`, `dy`) points from `start`
    /// (the frame when the drag began), kept on the stage and not smaller
    /// than 48 pt.
    pub(crate) fn resize(&mut self, corner: Corner, start: Rect, dx: f32, dy: f32) {
        let before = self.geometry();
        let (sw, sh) = self.stage;
        let (mut left, mut top) = (start.x, start.y);
        let (mut right, mut bottom) = (start.x + start.w, start.y + start.h);
        match corner {
            Corner::TopLeft | Corner::BottomLeft => {
                left = within(left + dx, FREE_EDGE_PT, right - FREE_MIN_PT);
            }
            Corner::TopRight | Corner::BottomRight => {
                right = within(right + dx, left + FREE_MIN_PT, sw - FREE_EDGE_PT);
            }
        }
        match corner {
            Corner::TopLeft | Corner::TopRight => {
                top = within(top + dy, FREE_EDGE_PT, bottom - FREE_MIN_PT);
            }
            Corner::BottomLeft | Corner::BottomRight => {
                bottom = within(bottom + dy, top + FREE_MIN_PT, sh - FREE_EDGE_PT);
            }
        }
        self.free = Some(Rect {
            x: left,
            y: top,
            w: right - left,
            h: bottom - top,
        });
        // The picture stays where it is, at its scale: the corner cuts the
        // side it is dragged on. It grows only when the frame outgrows it.
        let frame = self.frame();
        let (rw, rh) = self.turned_px();
        let base = (frame.w / rw).max(frame.h / rh);
        self.zoom = (before.scale / base).clamp(ZOOM_RANGE.0, ZOOM_RANGE.1);
        let scale = base * self.zoom;
        let (cx, cy) = (
            before.picture.x + before.picture.w / 2.0,
            before.picture.y + before.picture.h / 2.0,
        );
        // A grown picture grows about the frame's centre.
        let grow = scale / before.scale;
        let (fx, fy) = (frame.x + frame.w / 2.0, frame.y + frame.h / 2.0);
        self.centre = Some((fx + (cx - fx) * grow, fy + (cy - fy) * grow));
        self.pan(0.0, 0.0);
    }

    /// Whether the picture goes as it is: its own shape, uncut, unturned.
    pub(crate) fn untouched(&self) -> bool {
        self.rect().is_none() && self.turns == 0
    }

    /// The part kept, in the picture's pixels after its turns; none when it
    /// is the whole picture.
    pub(crate) fn rect(&self) -> Option<CropRect> {
        let geometry = self.geometry();
        let (rw, rh) = self.turned_px();
        let to_px = |points: f32| points / geometry.scale;
        // The edges are rounded, then the sides taken between them, so the
        // part never passes the picture by a rounding.
        let left = to_px(geometry.frame.x - geometry.picture.x)
            .clamp(0.0, rw)
            .round();
        let top = to_px(geometry.frame.y - geometry.picture.y)
            .clamp(0.0, rh)
            .round();
        let right = (left + to_px(geometry.frame.w)).min(rw).round();
        let bottom = (top + to_px(geometry.frame.h)).min(rh).round();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "pixel coordinates within the picture"
        )]
        let rect = CropRect {
            x: left as u32,
            y: top as u32,
            width: ((right - left) as u32).max(1),
            height: ((bottom - top) as u32).max(1),
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "picture sides"
        )]
        let whole = rect.x == 0
            && rect.y == 0
            && rect.width >= rw.round() as u32
            && rect.height >= rh.round() as u32;
        (!whole).then_some(rect)
    }
}

/// `value` kept between `low` and `high`; when rounding or a shape too
/// thin leaves no room between them, their middle. Never panics, unlike
/// `f32::clamp`.
fn within(value: f32, low: f32, high: f32) -> f32 {
    if low.is_nan() || high.is_nan() {
        return value;
    }
    if low > high {
        return low.midpoint(high);
    }
    value.clamp(low, high)
}

#[cfg(test)]
mod tests;
