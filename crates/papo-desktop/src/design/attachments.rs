//! The measures of attaching (`docs/design.md` §14.1, §14.2): the tray's
//! tiles and their marks, the drop card. Colours are the palette's.

use super::{Font, Radius, TYPE_BODY_PT, TextStyle, WEIGHT_SEMIBOLD};

/// A media tile's side, and every tile's height, in points.
pub(crate) const TILE_PT: f32 = 72.0;
/// A file tile's width range, in points.
pub(crate) const FILE_TILE_WIDTH_PT: (f32, f32) = (160.0, 220.0);
/// A file tile's insets, left and right, in points; the right one leaves
/// room for ✕.
pub(crate) const FILE_TILE_INSET_PT: (f32, f32) = (10.0, 30.0);
/// The extension block's side on a file tile, in points.
pub(crate) const EXTENSION_BLOCK_PT: f32 = 38.0;
/// The extension block's corner.
pub(crate) const EXTENSION_BLOCK_RADIUS: Radius = Radius(7.0);
/// The extension's type: mono 600 at about 9 pt.
pub(crate) const TYPE_EXTENSION: Font = {
    let size = TYPE_BODY_PT * 9.0 / 14.0;
    Font(TextStyle::new(size, WEIGHT_SEMIBOLD).lh(size)).mono()
};
/// A tile's ✕: the disc's side and its inset from the tile's corner, and
/// the cross inside it, in points.
pub(crate) const TILE_REMOVE_PT: f32 = 22.0;
pub(crate) const TILE_REMOVE_INSET_PT: f32 = 4.0;
pub(crate) const TILE_REMOVE_ICON_PT: f32 = 12.0;
/// How long ✕ takes to fade in or out, in seconds (none under reduced
/// motion, which Rune applies).
pub(crate) const TILE_REMOVE_FADE_S: f32 = 0.12;
/// How long a removed tile takes to shrink out, in seconds.
pub(crate) const TILE_EXIT_S: f32 = 0.16;
/// The scale a removed tile shrinks to.
pub(crate) const TILE_EXIT_SCALE: f32 = 0.85;
/// A media badge ("GIF") and its inset from the tile's corner, in points.
pub(crate) const TILE_BADGE_INSET_PT: f32 = 4.0;
pub(crate) const MEDIA_BADGE_PAD_PT: (f32, f32) = (6.0, 1.0);
pub(crate) const MEDIA_BADGE_RADIUS: Radius = Radius(5.0);
/// The black behind a media badge and a preparing tile's ring.
pub(crate) const MEDIA_BADGE_ALPHA: f32 = 0.6;
pub(crate) const PREPARING_SCRIM_ALPHA: f32 = 0.4;
/// A preparing tile's ring, in points.
pub(crate) const PREPARING_RING_PT: f32 = 20.0;
/// The type of a media badge: mono 600 at about 10.5 pt.
pub(crate) const TYPE_MEDIA_BADGE: Font = {
    let size = TYPE_BODY_PT * 10.5 / 14.0;
    Font(TextStyle::new(size, WEIGHT_SEMIBOLD).lh(16.0 / 10.5 * size)).mono()
};
/// A dragged tile's opacity, and the width of the line where it lands.
pub(crate) const DRAGGED_TILE_OPACITY: f32 = 0.4;
pub(crate) const DROP_SLOT_PT: f32 = 2.0;
/// The window's dimming while files are dragged over it.
pub(crate) const DROP_DIM_ALPHA: f32 = 0.45;
/// The drop card's paperclip, its dashed edge (width, dash, gap) and its
/// padding, in points.
pub(crate) const DROP_ICON_PT: f32 = 32.0;
pub(crate) const DROP_EDGE_PT: (f32, f32, f32) = (2.0, 6.0, 4.0);
pub(crate) const DROP_CARD_PAD_PT: (f32, f32) = (32.0, 24.0);

/// `radius.card`: media in messages, file cards, albums.
pub(crate) const RADIUS_CARD: Radius = Radius(10.0);
/// The media box's limits at the Medium size (§14.9; the Media settings
/// come with D7b), in points.
pub(crate) const MEDIA_MAX_PT: (f32, f32) = (448.0, 400.0);
/// The share of the timeline's height a media box may take (§14.4).
pub(crate) const MEDIA_MAX_HEIGHT_SHARE: f32 = 0.55;
/// The box of media whose shape is unknown, width over height.
pub(crate) const MEDIA_UNKNOWN_ASPECT: f32 = 16.0 / 9.0;
/// Between an album's cells, in points.
pub(crate) const ALBUM_GAP_PT: f32 = 2.0;
/// An album's shape, width over height: two items, three or more.
pub(crate) const ALBUM_ASPECT: (f32, f32) = (3.0 / 2.0, 4.0 / 3.0);
/// Cells an album draws; the rest are counted on the last ("+N").
pub(crate) const ALBUM_CELLS: usize = 4;
/// The black over an album's last cell when more follow.
pub(crate) const ALBUM_MORE_ALPHA: f32 = 0.6;
/// An uploading item's ring: its side and the track's white.
pub(crate) const UPLOAD_RING_PT: f32 = 36.0;
pub(crate) const UPLOAD_RING_TRACK_ALPHA: f32 = 0.3;
/// The black over an uploading item.
pub(crate) const UPLOAD_SCRIM_ALPHA: f32 = 0.4;
/// The ✕ inside the ring, in points.
pub(crate) const UPLOAD_CANCEL_ICON_PT: f32 = 14.0;
/// A file card: its widest, its padding (left, right), and the accent
/// bar along its bottom while it uploads, in points.
pub(crate) const FILE_CARD_MAX_PT: f32 = 448.0;
pub(crate) const FILE_CARD_PAD_PT: (f32, f32) = (10.0, 6.0);
pub(crate) const FILE_CARD_BAR_PT: f32 = 2.0;
/// A file card's end button, in points.
pub(crate) const FILE_CARD_BUTTON_PT: f32 = 38.0;
