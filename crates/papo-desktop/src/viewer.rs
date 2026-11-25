//! The viewer (`docs/design.md` §14.8): a message's pictures over the whole window, one at a
//! time, ‹ › between them (no wrap), opened fitted; the wheel zooms toward the pointer (×1.25 a
//! notch, from fit to 800 %), a drag pans once the picture is larger than the room, a double
//! click switches between fit and 100 % at the pointer, and the keys of §12.
//!
//! Owns the state and its geometry, pure and tested; loading the full picture, the keys' wiring
//! and Jump to message are `shell::viewer`'s, the drawing `viewer::view`'s.

use rune::Image;

use crate::media_store::MediaKey;
use crate::message_media::received::MediaAt;

pub(crate) mod video;
pub(crate) mod view;

/// One notch of zoom.
const ZOOM_STEP: f32 = 1.25;

/// The most a picture is zoomed: 800 %.
const MAX_ZOOM: f32 = 8.0;

/// A wheel's travel per notch for a trackpad's point-precise deltas.
const WHEEL_NOTCH_PT: f32 = 40.0;

/// Room kept clear above the picture for the pills, and at its sides and below, in points.
pub(crate) const CLEAR_TOP_PT: f32 = 64.0;
pub(crate) const CLEAR_SIDE_PT: f32 = 24.0;

/// One picture the viewer steps through.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ViewerItem {
    /// Its place among the message's attachments.
    pub(crate) index: usize,
    pub(crate) key: MediaKey,
    pub(crate) name: String,
    pub(crate) gif: bool,
    /// A video: it plays here (`shell::viewer_video`), and has no whole picture to fetch.
    pub(crate) video: bool,
}

/// The open viewer.
pub(crate) struct Viewer {
    /// The message, and the attachment opened first.
    pub(crate) at: MediaAt,
    pub(crate) items: Vec<ViewerItem>,
    /// Which item shows.
    pub(crate) current: usize,
    /// Display pixels per picture pixel; none while fitted.
    zoom: Option<f32>,
    /// The picture's centre from the room's centre, in points.
    offset: [f32; 2],
    /// Where a drag started: the pointer and the offset then.
    drag_from: Option<([f32; 2], [f32; 2])>,
    /// The current item's whole picture, once fetched (decoding or decoded).
    pub(crate) full: Option<Image>,
    /// When the full picture was asked for, on the window's clock: its ring shows after 0.5 s.
    pub(crate) asked_s: Option<f64>,
    /// The ring shows: the full picture has taken more than half a second.
    pub(crate) ring: bool,
    /// The author and when, as the pill says them.
    pub(crate) author: String,
    pub(crate) when: String,
    /// The window went full screen from the viewer.
    pub(crate) full_screen: bool,
    /// The whole picture could not be fetched or decoded: Retry asks again.
    pub(crate) failed: bool,
}

impl Viewer {
    /// The viewer on `items`, showing the one at attachment `at.index`.
    pub(crate) fn new(at: MediaAt, items: Vec<ViewerItem>, author: String, when: String) -> Viewer {
        let current = items
            .iter()
            .position(|item| item.index == at.index)
            .unwrap_or_default();
        Viewer {
            at,
            items,
            current,
            zoom: None,
            offset: [0.0, 0.0],
            drag_from: None,
            full: None,
            asked_s: None,
            ring: false,
            author,
            when,
            full_screen: false,
            failed: false,
        }
    }

    /// The element that gets the keyboard back on close: the item showing, or in an album the
    /// last cell drawn ("+N") for one past it.
    pub(crate) fn return_focus(&self) -> String {
        let shown = self.current.min(papo_sdk_ui::ALBUM_CELLS - 1);
        let index = self
            .items
            .get(shown)
            .map_or(self.at.index, |item| item.index);
        crate::message_media::received::media_element_key(MediaAt { index, ..self.at })
    }

    /// The item showing.
    pub(crate) fn item(&self) -> Option<&ViewerItem> {
        self.items.get(self.current)
    }

    /// Steps to the previous (`-1`) or next (`1`) item, if there is one: fitted, its picture
    /// asked for again. Returns whether it moved.
    pub(crate) fn step(&mut self, by: isize) -> bool {
        let Some(next) = self.current.checked_add_signed(by) else {
            return false;
        };
        if next >= self.items.len() {
            return false;
        }
        self.current = next;
        self.fit();
        self.full = None;
        self.asked_s = None;
        self.ring = false;
        self.failed = false;
        true
    }

    /// Back to fit.
    pub(crate) fn fit(&mut self) {
        self.zoom = None;
        self.offset = [0.0, 0.0];
        self.drag_from = None;
    }

    /// Where the picture of `natural` pixels draws in a room of `room` points: its centre's
    /// offset from the room's centre and its size, in points.
    pub(crate) fn placed(&self, natural: [f32; 2], room: [f32; 2]) -> Placed {
        let scale = self.scale(natural, room);
        Placed {
            offset: self.offset,
            size: [natural[0] * scale, natural[1] * scale],
            zoomed: self
                .zoom
                .is_some_and(|zoom| zoom > fit_scale(natural, room) + 1e-4),
            scale,
        }
    }

    /// The display scale now.
    pub(crate) fn scale(&self, natural: [f32; 2], room: [f32; 2]) -> f32 {
        self.zoom.unwrap_or_else(|| fit_scale(natural, room))
    }

    /// Zooms by `factor` toward `toward` (from the room's centre, in points), between fit and
    /// 800 %.
    pub(crate) fn zoom_by(
        &mut self,
        factor: f32,
        toward: [f32; 2],
        natural: [f32; 2],
        room: [f32; 2],
    ) {
        let old = self.scale(natural, room);
        let new = (old * factor).clamp(fit_scale(natural, room), MAX_ZOOM);
        self.zoom_to(new, toward, natural, room);
    }

    /// The wheel moved `dy` (`precise` for a trackpad's points) over `toward`.
    pub(crate) fn wheel(
        &mut self,
        dy: f32,
        precise: bool,
        toward: [f32; 2],
        natural: [f32; 2],
        room: [f32; 2],
    ) {
        if dy == 0.0 || !dy.is_finite() {
            return;
        }
        // Content moving down (positive) is the wheel turned toward the reader: zoom in.
        let notches = if precise {
            dy / WHEEL_NOTCH_PT
        } else {
            dy.signum()
        };
        self.zoom_by(ZOOM_STEP.powf(notches), toward, natural, room);
    }

    /// One step in (`+`) or out (`−`), toward the centre.
    pub(crate) fn zoom_step(&mut self, inward: bool, natural: [f32; 2], room: [f32; 2]) {
        let factor = if inward { ZOOM_STEP } else { 1.0 / ZOOM_STEP };
        self.zoom_by(factor, [0.0, 0.0], natural, room);
    }

    /// A double click at `toward`: fit when zoomed, else 100 % there.
    pub(crate) fn double_click(&mut self, toward: [f32; 2], natural: [f32; 2], room: [f32; 2]) {
        if self.placed(natural, room).zoomed {
            self.fit();
        } else {
            self.zoom_to(1.0_f32.max(fit_scale(natural, room)), toward, natural, room);
        }
    }

    fn zoom_to(&mut self, new: f32, toward: [f32; 2], natural: [f32; 2], room: [f32; 2]) {
        let old = self.scale(natural, room);
        if (new - fit_scale(natural, room)).abs() < 1e-4 {
            self.fit();
            return;
        }
        let ratio = new / old;
        self.offset = [
            toward[0] - (toward[0] - self.offset[0]) * ratio,
            toward[1] - (toward[1] - self.offset[1]) * ratio,
        ];
        self.zoom = Some(new);
        self.clamp(natural, room);
    }

    /// A drag at `pointer` (window points): pans once the picture is larger than the room.
    pub(crate) fn drag(
        &mut self,
        pointer: [f32; 2],
        start: bool,
        natural: [f32; 2],
        room: [f32; 2],
    ) {
        if !self.placed(natural, room).zoomed {
            return;
        }
        if start || self.drag_from.is_none() {
            self.drag_from = Some((pointer, self.offset));
            return;
        }
        if let Some((from, offset)) = self.drag_from {
            self.offset = [
                offset[0] + pointer[0] - from[0],
                offset[1] + pointer[1] - from[1],
            ];
            self.clamp(natural, room);
        }
    }

    /// The drag ended.
    pub(crate) fn drag_end(&mut self) {
        self.drag_from = None;
    }

    /// Keeps the picture over the room: it moves only as far as it is larger.
    fn clamp(&mut self, natural: [f32; 2], room: [f32; 2]) {
        let scale = self.scale(natural, room);
        for axis in 0..2 {
            let room_left = ((natural[axis] * scale - room[axis]) / 2.0).max(0.0);
            self.offset[axis] = self.offset[axis].clamp(-room_left, room_left);
        }
    }

    /// The zoom as a whole percentage: "100 %".
    pub(crate) fn percent(&self, natural: [f32; 2], room: [f32; 2]) -> u32 {
        let percent = (self.scale(natural, room) * 100.0).round();
        if percent.is_finite() && percent >= 0.0 {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a finite, non-negative percentage up to 800"
            )]
            let percent = percent as u32;
            percent
        } else {
            0
        }
    }
}

/// Where the picture draws.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placed {
    /// Its centre from the room's centre, in points.
    pub(crate) offset: [f32; 2],
    /// Its size, in points.
    pub(crate) size: [f32; 2],
    /// It is larger than fit: a drag pans it.
    pub(crate) zoomed: bool,
    pub(crate) scale: f32,
}

/// The scale at which `natural` fits `room`, never above 100 %.
fn fit_scale(natural: [f32; 2], room: [f32; 2]) -> f32 {
    // Before the first layout there is no room to fit: 100 % until there is.
    if natural[0] <= 0.0 || natural[1] <= 0.0 || room[0] <= 0.0 || room[1] <= 0.0 {
        return 1.0;
    }
    (room[0] / natural[0])
        .min(room[1] / natural[1])
        .clamp(1e-3, 1.0)
}

#[cfg(test)]
mod tests;
