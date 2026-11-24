//! The settings panel's measures (`docs/design.md` §8; geometry from the
//! mock's Settings page): the panel and its nav, a page's head and body,
//! the row tiles, and the controls rows end in. Values in points at the
//! defaults; what scales is typed so (`Space`, `Radius`).

use rune::Color;

use super::{Radius, Space};

/// The panel's widest, in points: past it, it is centred in the window.
pub(crate) const PANEL_MAX_WIDTH_PT: f32 = 960.0;
/// The panel's corner radius.
pub(crate) const PANEL_RADIUS: Radius = Radius(18.0);
/// The nav column's width, in points.
pub(crate) const NAV_WIDTH_PT: f32 = 280.0;
/// The nav column's padding.
pub(crate) const NAV_PADDING: Space = Space(12.0);
/// A nav row's vertical padding, and its horizontal padding in points.
pub(crate) const NAV_ROW_PADDING: Space = Space(7.0);
pub(crate) const NAV_ROW_INSET_PT: f32 = 10.0;
/// The circle holding a nav row's icon, in points across.
pub(crate) const NAV_ICON_CIRCLE_PT: f32 = 36.0;
/// The search field's height.
pub(crate) const SEARCH_HEIGHT: Space = Space(40.0);
/// A page head's least height, in points, and its padding (top, right,
/// bottom, left).
pub(crate) const HEAD_MIN_HEIGHT_PT: f32 = 58.0;
pub(crate) const HEAD_PADDING_PT: [f32; 4] = [14.0, 14.0, 8.0, 24.0];
/// The round ← and ✕ in a page's head, in points across.
pub(crate) const HEAD_BUTTON_PT: f32 = 32.0;
/// A page body's padding (top, sides, bottom), in points.
pub(crate) const BODY_PADDING_PT: (f32, f32, f32) = (6.0, 24.0, 28.0);
/// Between a page's groups, and between a group's title and its tiles.
pub(crate) const GROUP_GAP: Space = Space(22.0);
pub(crate) const GROUP_TITLE_GAP: Space = Space(8.0);
/// A row tile's least height and its vertical padding; its horizontal
/// padding in points.
pub(crate) const ROW_MIN_HEIGHT: Space = Space(54.0);
pub(crate) const ROW_PADDING: Space = Space(9.0);
pub(crate) const ROW_INSET_PT: f32 = 16.0;
/// A group's outer corners, and the corners between its tiles (§8: 16 and
/// 4).
pub(crate) const TILE_OUTER_RADIUS: Radius = Radius(16.0);
pub(crate) const TILE_INNER_RADIUS: Radius = Radius(4.0);
/// A switch: its track (width, height) and its knob, in points.
pub(crate) const SWITCH_PT: (f32, f32) = (38.0, 22.0);
pub(crate) const SWITCH_KNOB_PT: f32 = 16.0;
/// A slider: its track's height and its thumb's side, in points.
pub(crate) const SLIDER_TRACK_PT: f32 = 22.0;
pub(crate) const SLIDER_THUMB_PT: f32 = 28.0;
/// The ring round a slider's thumb, in points.
pub(crate) const SLIDER_THUMB_RING_PT: f32 = 3.0;
/// A switch's knob, a slider thumb's ring and an unchosen colour dot's
/// ring: white in every theme, as the mock draws them. They carry no text,
/// so no contrast pairing applies.
pub(crate) const KNOB: Color = Color::WHITE;
/// The soft shadow under a slider's thumb and a colour dot: black at 30 %,
/// so a white knob still shows on a light tile.
pub(crate) const KNOB_SHADOW: Color = Color {
    r: 0.0,
    g: 0.0,
    b: 0.0,
    a: 0.3,
};
/// An accent swatch, in points square, its radius, and the ring that marks
/// the chosen one.
pub(crate) const SWATCH_PT: f32 = 26.0;
pub(crate) const SWATCH_RADIUS: Radius = Radius(8.0);
pub(crate) const SWATCH_RING_PT: f32 = 2.0;
/// A starting-point chip's height.
pub(crate) const CHIP_HEIGHT: Space = Space(34.0);
/// The colour field's height, in points.
pub(crate) const FIELD_HEIGHT_PT: f32 = 170.0;
/// The field's hue dots: their grid step and size, in points.
pub(crate) const FIELD_DOT_STEP_PT: f32 = 11.0;
pub(crate) const FIELD_DOT_PT: f32 = 2.4;
/// A background colour on the field, in points across, and its ring.
pub(crate) const COLOUR_DOT_PT: f32 = 22.0;
pub(crate) const COLOUR_DOT_RING_PT: f32 = 3.0;
/// A preset swatch, in points square.
pub(crate) const PRESET_PT: f32 = 30.0;
/// The field's tool buttons (+, −, harmony), in points square.
pub(crate) const TOOL_PT: f32 = 30.0;
