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
