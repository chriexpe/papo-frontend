//! The measures of link cards (`docs/design.md` §15.1, §15.3): the rule, the provider line and
//! its letter tile, the fields' grid and the provider poster's chip and play disc. Colours are
//! the palette's; the rule's fitting to its floor is `floors`'.

use rune::TextStyle;

use super::{Font, TYPE_BODY_PT, WEIGHT_SEMIBOLD};

/// The rule on a card's left edge, in points.
pub(crate) const CARD_RULE_PT: f32 = 2.0;
/// The contrast the rule keeps on the message list (§15.1).
pub(crate) const CARD_RULE_FLOOR: f32 = 3.0;
/// Between the provider line's tile and its words, in points.
pub(crate) const PROVIDER_LINE_GAP_PT: f32 = 6.0;
/// The provider's letter tile, standing for the favicon the server does not relay.
pub(crate) const LETTER_TILE_PT: f32 = 12.0;
pub(crate) const LETTER_TILE_RADIUS_PT: f32 = 4.0;
/// The letter on a tile: 600 at 8 pt, a fixed size like the tile's.
pub(crate) const TYPE_LETTER_TILE: Font = {
    let size = TYPE_BODY_PT * 8.0 / 14.0;
    Font(TextStyle::new(size, WEIGHT_SEMIBOLD).lh(size))
};
/// Lines a description shows before its ellipsis.
pub(crate) const CARD_DESCRIPTION_LINES: usize = 2;
/// Fields to a row, the gaps between rows and between columns, and the lines a value takes.
pub(crate) const CARD_FIELDS_PER_ROW: usize = 3;
pub(crate) const CARD_FIELD_ROW_GAP_PT: f32 = 6.0;
pub(crate) const CARD_FIELD_COLUMN_GAP_PT: f32 = 14.0;
pub(crate) const CARD_FIELD_VALUE_LINES: usize = 2;
/// The chip naming whose player a poster is, at its top left: its height, the padding at its
/// start and end, the gap after its tile, and its inset from the box's corner.
pub(crate) const PROVIDER_CHIP_PT: f32 = 24.0;
pub(crate) const PROVIDER_CHIP_PAD_PT: (f32, f32) = (7.0, 9.0);
pub(crate) const PROVIDER_CHIP_GAP_PT: f32 = 6.0;
pub(crate) const PROVIDER_CHIP_INSET_PT: f32 = 8.0;
/// The poster's play disc darkens to this under the pointer (50 % at rest, as a video's).
pub(crate) const PROVIDER_PLAY_HOVER_ALPHA: f32 = 0.66;
/// A late preview's growth (§15.1), none under reduced motion.
pub(crate) const PREVIEW_GROW_S: f32 = 0.16;
/// A provider poster under the ask or loading dims to 42 % (§15.3): black over it at this.
pub(crate) const PLAYER_DIM_ALPHA: f32 = 0.58;
/// The ask's panel of dark glass at the box's foot: its near-black, its blur, its padding and
/// the gap between its words and its buttons.
pub(crate) const PLAYER_ASK_RGB: [u8; 3] = [14, 16, 22];
pub(crate) const PLAYER_ASK_ALPHA: f32 = 0.84;
pub(crate) const PLAYER_ASK_BLUR_PT: f32 = 14.0;
pub(crate) const PLAYER_ASK_PAD_PT: f32 = 12.0;
pub(crate) const PLAYER_ASK_GAP_PT: f32 = 9.0;
/// The ask's and a failure's buttons: height, side padding and the gap between them; the
/// ask's second button is white at this, its third's words white at the other.
pub(crate) const PLAYER_BUTTON_PT: f32 = 30.0;
pub(crate) const PLAYER_BUTTON_PAD_PT: f32 = 12.0;
pub(crate) const PLAYER_BUTTON_GAP_PT: f32 = 8.0;
pub(crate) const PLAYER_BUTTON_SECONDARY_ALPHA: f32 = 0.14;
pub(crate) const PLAYER_BUTTON_GHOST_ALPHA: f32 = 0.82;
/// Loading: a 36 pt disc with its turning ring, shown after half a second.
pub(crate) const PLAYER_RING_PT: f32 = 36.0;
pub(crate) const PLAYER_RING_ICON_PT: f32 = 22.0;
pub(crate) const PLAYER_RING_PERIOD_S: f64 = 0.9;
pub(crate) const PLAYER_RING_AFTER_S: f64 = 0.5;
/// A live player's two glass buttons at its top right: size, the gap between them, their
/// near-black, its alpha at rest and under the pointer, and the blur behind.
pub(crate) const PLAYER_GLASS_PT: f32 = 28.0;
pub(crate) const PLAYER_GLASS_GAP_PT: f32 = 6.0;
pub(crate) const PLAYER_GLASS_RGB: [u8; 3] = [10, 12, 18];
pub(crate) const PLAYER_GLASS_ALPHA: f32 = 0.58;
pub(crate) const PLAYER_GLASS_HOVER_ALPHA: f32 = 0.8;
pub(crate) const PLAYER_GLASS_BLUR_PT: f32 = 8.0;
pub(crate) const PLAYER_GLASS_ICON_PT: f32 = 16.0;
/// A failed player's icon and the gap between its parts.
pub(crate) const PLAYER_FAIL_ICON_PT: f32 = 20.0;
pub(crate) const PLAYER_FAIL_GAP_PT: f32 = 8.0;
/// The note on a released player's poster, a badge at its foot's start: its padding.
pub(crate) const PLAYER_NOTE_PAD_PT: (f32, f32) = (8.0, 2.0);
/// A track's card (§15.4): its cover, the play disc on it and the disc's icon, the gap to the
/// title beside it, and the strip its player plays in.
pub(crate) const PLAYER_COVER_PT: f32 = 96.0;
pub(crate) const PLAYER_COVER_DISC_PT: f32 = 36.0;
pub(crate) const PLAYER_COVER_ICON_PT: f32 = 18.0;
pub(crate) const PLAYER_COVER_GAP_PT: f32 = 12.0;
pub(crate) const PLAYER_STRIP_PT: f32 = 152.0;
