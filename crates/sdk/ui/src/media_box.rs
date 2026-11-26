//! The box a message's pictures take before anything loads (`docs/design.md` §14.4, §14.9), so
//! a row never changes height when its media arrive: what a picture's size is known as, the
//! bounds its host and the size setting give, and the box fitted into them; the album layouts
//! are `album`'s.
//! Owns only the rules; decoding, drawing, the settings' storage and their words are the
//! client's.
//!
//! Sizes are in points (the desktop's points, Android's dp), and a picture's pixel is drawn as
//! one point, so the text size never changes media. Every box is rounded to whole points, so
//! the same inputs give the same row on every frame and on both sides of a send.

use std::ops::RangeInclusive;

use papo_sdk::Attachment;

use crate::attachment_kind::AttachmentKind;

mod album;
pub use album::{ALBUM_CELLS, ALBUM_GAP_PT, AlbumCell, AlbumLayout};

/// The long edge, in pixels, under which the server never shrinks a picture for its thumbnail:
/// it rounds a thumbnail's long edge down to a multiple of this, and leaves a smaller picture
/// as it is, so a thumbnail under it has the picture's own size.
const SERVER_THUMBNAIL_STEP_PX: u32 = 128;

/// The share of its host's height media may take: a timeline's, or a panel's (§14.4, §10).
pub const HOST_HEIGHT_SHARE: f32 = 0.55;

/// The share of the window's height media may take in a dialog (§10).
pub const DIALOG_HEIGHT_SHARE: f32 = 1.0 / 3.0;

/// What a picture's size is known as, in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaSize {
    /// Its own size: it is never drawn larger.
    Exact {
        /// Width in pixels, never zero.
        width_px: u32,
        /// Height in pixels, never zero.
        height_px: u32,
    },
    /// Only its shape: the server's thumbnail of a larger picture. It fills its box.
    Shape {
        /// Width of the shape, never zero.
        width_px: u32,
        /// Height of the shape, never zero.
        height_px: u32,
    },
    /// Nothing: the box is 16:9 and the picture is letterboxed in it.
    Unknown,
}

impl MediaSize {
    /// A picture's own size; unknown when a side is zero.
    #[must_use]
    pub fn exact(width_px: u32, height_px: u32) -> MediaSize {
        if width_px == 0 || height_px == 0 {
            return MediaSize::Unknown;
        }
        MediaSize::Exact {
            width_px,
            height_px,
        }
    }

    /// What a received attachment says of its size: its own `width` and `height` when the
    /// server sent them (videos), else a picture's thumbnail, which is its own size under the
    /// server's thumbnail step and only its shape above it.
    #[must_use]
    pub fn of(attachment: &Attachment) -> MediaSize {
        let side = |value: Option<i64>| {
            value
                .and_then(|value| u32::try_from(value).ok())
                .filter(|value| *value > 0)
        };
        if let (Some(width_px), Some(height_px)) = (side(attachment.width), side(attachment.height))
        {
            return MediaSize::Exact {
                width_px,
                height_px,
            };
        }
        if !AttachmentKind::of(attachment).is_picture() {
            return MediaSize::Unknown;
        }
        match (
            side(attachment.thumbnail_width),
            side(attachment.thumbnail_height),
        ) {
            (Some(width_px), Some(height_px))
                if width_px.max(height_px) < SERVER_THUMBNAIL_STEP_PX =>
            {
                MediaSize::Exact {
                    width_px,
                    height_px,
                }
            }
            (Some(width_px), Some(height_px)) => MediaSize::Shape {
                width_px,
                height_px,
            },
            _ => MediaSize::Unknown,
        }
    }
}

/// The widths a Custom size may take, in points (Embed size's too).
pub const CUSTOM_WIDTH_PT: RangeInclusive<u16> = 240..=720;

/// The heights a Custom size may take, in points.
pub const CUSTOM_HEIGHT_PT: RangeInclusive<u16> = 200..=640;

/// The step of a Custom size's sliders, in points.
pub const CUSTOM_STEP_PT: u16 = 8;

/// A media size setting (Image size, Video size): the largest box media may take before the
/// host's own limits. Embed size is [`EmbedSizeSetting`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MediaSizeSetting {
    /// 320 × 280.
    Small,
    /// 448 × 400, the default.
    #[default]
    Medium,
    /// 600 × 520.
    Large,
    /// The sizes chosen on its sliders; build it with [`MediaSizeSetting::custom`].
    Custom {
        /// Max width in points, within [`CUSTOM_WIDTH_PT`] on a [`CUSTOM_STEP_PT`] step.
        max_width_pt: u16,
        /// Max height in points, within [`CUSTOM_HEIGHT_PT`] on a [`CUSTOM_STEP_PT`] step.
        max_height_pt: u16,
    },
}

impl MediaSizeSetting {
    /// A Custom size, each side brought into its range and onto the sliders' step.
    #[must_use]
    pub fn custom(max_width_pt: u16, max_height_pt: u16) -> MediaSizeSetting {
        MediaSizeSetting::Custom {
            max_width_pt: on_step(max_width_pt, &CUSTOM_WIDTH_PT),
            max_height_pt: on_step(max_height_pt, &CUSTOM_HEIGHT_PT),
        }
    }

    /// Its max width and max height, in points. Custom's sides are brought into their ranges,
    /// whatever was stored.
    #[must_use]
    pub fn max_size_pt(self) -> (u16, u16) {
        match self {
            MediaSizeSetting::Small => (320, 280),
            MediaSizeSetting::Medium => (448, 400),
            MediaSizeSetting::Large => (600, 520),
            MediaSizeSetting::Custom {
                max_width_pt,
                max_height_pt,
            } => (
                on_step(max_width_pt, &CUSTOM_WIDTH_PT),
                on_step(max_height_pt, &CUSTOM_HEIGHT_PT),
            ),
        }
    }
}

/// The heights Embed size's Custom may take, in points.
pub const EMBED_CUSTOM_HEIGHT_PT: RangeInclusive<u16> = 160..=640;

/// Embed size (`docs/design.md` §15.7): the largest box of a link card's picture and of a
/// provider's player, apart from Image size and Video size.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum EmbedSizeSetting {
    /// 320 × 240.
    Small,
    /// 448 × 336, the default.
    #[default]
    Medium,
    /// 600 × 450.
    Large,
    /// The sizes chosen on its sliders; build it with [`EmbedSizeSetting::custom`].
    Custom {
        /// Max width in points, within [`CUSTOM_WIDTH_PT`] on a [`CUSTOM_STEP_PT`] step.
        max_width_pt: u16,
        /// Max height in points, within [`EMBED_CUSTOM_HEIGHT_PT`] on a [`CUSTOM_STEP_PT`]
        /// step.
        max_height_pt: u16,
    },
}

impl EmbedSizeSetting {
    /// A Custom size, each side brought into its range and onto the sliders' step.
    #[must_use]
    pub fn custom(max_width_pt: u16, max_height_pt: u16) -> EmbedSizeSetting {
        EmbedSizeSetting::Custom {
            max_width_pt: on_step(max_width_pt, &CUSTOM_WIDTH_PT),
            max_height_pt: on_step(max_height_pt, &EMBED_CUSTOM_HEIGHT_PT),
        }
    }

    /// Its max width and max height, in points. Custom's sides are brought into their ranges,
    /// whatever was stored.
    #[must_use]
    pub fn max_size_pt(self) -> (u16, u16) {
        match self {
            EmbedSizeSetting::Small => (320, 240),
            EmbedSizeSetting::Medium => (448, 336),
            EmbedSizeSetting::Large => (600, 450),
            EmbedSizeSetting::Custom {
                max_width_pt,
                max_height_pt,
            } => (
                on_step(max_width_pt, &CUSTOM_WIDTH_PT),
                on_step(max_height_pt, &EMBED_CUSTOM_HEIGHT_PT),
            ),
        }
    }
}

/// `value` brought into `range` and onto the step counted from its start.
fn on_step(value: u16, range: &RangeInclusive<u16>) -> u16 {
    let (low, high) = (*range.start(), *range.end());
    let clamped = value.clamp(low, high);
    let steps = (clamped - low + CUSTOM_STEP_PT / 2) / CUSTOM_STEP_PT;
    (low + steps * CUSTOM_STEP_PT).min(high)
}

/// The largest box media may take in a host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaBounds {
    /// Max width in points, at least one.
    pub max_width_pt: f32,
    /// Max height in points, at least one.
    pub max_height_pt: f32,
}

impl MediaBounds {
    /// The bounds of `setting` in a host whose content column is `column_width_pt` wide and
    /// whose media may be `height_cap_pt` tall ([`HOST_HEIGHT_SHARE`] of a timeline's or a
    /// panel's height, [`DIALOG_HEIGHT_SHARE`] of a dialog's window). A limit that is not a
    /// number limits nothing; a side that comes out below one point is one point.
    #[must_use]
    pub fn new(setting: MediaSizeSetting, column_width_pt: f32, height_cap_pt: f32) -> MediaBounds {
        MediaBounds::of_max_size(setting.max_size_pt(), column_width_pt, height_cap_pt)
    }

    /// The bounds of a setting's `(max width, max height)` in points (Embed size's
    /// [`EmbedSizeSetting::max_size_pt`], say), in a host as [`MediaBounds::new`] takes it.
    #[must_use]
    pub fn of_max_size(
        (width_pt, height_pt): (u16, u16),
        column_width_pt: f32,
        height_cap_pt: f32,
    ) -> MediaBounds {
        MediaBounds {
            max_width_pt: at_least_one(f32::from(width_pt).min(column_width_pt)),
            max_height_pt: at_least_one(f32::from(height_pt).min(height_cap_pt)),
        }
    }
}

impl MediaBounds {
    /// These bounds with each side a finite number of at least one point, however they were
    /// built.
    pub(crate) fn sane(self) -> MediaBounds {
        MediaBounds {
            max_width_pt: at_least_one(self.max_width_pt),
            max_height_pt: at_least_one(self.max_height_pt),
        }
    }
}

fn at_least_one(value: f32) -> f32 {
    if value.is_finite() {
        value.max(1.0)
    } else {
        1.0
    }
}

/// How a picture sits in its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaFit {
    /// It covers the box, whose shape is its own (an album's cells are cropped to fill).
    Fill,
    /// Its shape is not known: it is fitted inside the box, on the code surface.
    Letterbox,
}

/// The box a picture takes, in whole points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MediaBox {
    /// Width in points, at least one.
    pub width_pt: f32,
    /// Height in points, at least one.
    pub height_pt: f32,
    /// How the picture sits in it.
    pub fit: MediaFit,
}

/// The box of one picture of `size` within `bounds`, keeping its shape: its own size or less
/// when it is known, filling the bounds when only its shape is, and a 16:9 box at the max width
/// (letterboxed) when nothing is.
#[must_use]
pub fn media_box(size: MediaSize, bounds: MediaBounds) -> MediaBox {
    let bounds = bounds.sane();
    match size {
        MediaSize::Exact {
            width_px,
            height_px,
        } => fitted(width_px, height_px, bounds, false),
        MediaSize::Shape {
            width_px,
            height_px,
        } => fitted(width_px, height_px, bounds, true),
        MediaSize::Unknown => {
            let (width_pt, height_pt) = largest_of_shape(16.0 / 9.0, bounds);
            MediaBox {
                width_pt,
                height_pt,
                fit: MediaFit::Letterbox,
            }
        }
    }
}

/// `width` × `height` scaled to fit `bounds`; enlarged to fill them only when `may_enlarge`.
fn fitted(width: u32, height: u32, bounds: MediaBounds, may_enlarge: bool) -> MediaBox {
    #[expect(
        clippy::cast_precision_loss,
        reason = "picture sides are far below 2^24"
    )]
    let (width, height) = (width as f32, height as f32);
    let mut scale = (bounds.max_width_pt / width).min(bounds.max_height_pt / height);
    if !may_enlarge {
        scale = scale.min(1.0);
    }
    MediaBox {
        width_pt: whole(width * scale, bounds.max_width_pt),
        height_pt: whole(height * scale, bounds.max_height_pt),
        fit: MediaFit::Fill,
    }
}

/// The largest box of `aspect` (width over height) within `bounds`, in whole points.
pub(crate) fn largest_of_shape(aspect: f32, bounds: MediaBounds) -> (f32, f32) {
    let mut width = bounds.max_width_pt;
    let mut height = width / aspect;
    if height > bounds.max_height_pt {
        height = bounds.max_height_pt;
        width = height * aspect;
    }
    (
        whole(width, bounds.max_width_pt),
        whole(height, bounds.max_height_pt),
    )
}

/// `value` rounded to a whole point, between one and `limit`.
fn whole(value: f32, limit: f32) -> f32 {
    value.round().min(limit.floor()).max(1.0)
}

#[cfg(test)]
mod tests;
