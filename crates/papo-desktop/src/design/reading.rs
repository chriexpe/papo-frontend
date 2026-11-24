//! The measures of the reading controls (`docs/design.md` §6, Reading
//! controls): the boundary lines over the rows, ⌛ and ↓, and ⌛'s pill.

use super::Space;

/// ⌛ and ↓: round buttons this many points across, holding an icon this
/// big, stacked this far apart and kept this far in from the timeline's
/// right and bottom edges.
pub(crate) const JUMP_BUTTON_PT: f32 = 40.0;
pub(crate) const JUMP_ICON_PT: f32 = 18.0;
pub(crate) const JUMP_GAP: Space = Space(8.0);
pub(crate) const JUMP_INSET: Space = Space(12.0);

/// A boundary's line: this thick, in the accent at this opacity, this far
/// from its chip.
pub(crate) const BOUNDARY_LINE_PT: f32 = 1.0;
pub(crate) const BOUNDARY_LINE_ALPHA: f32 = 0.5;
pub(crate) const BOUNDARY_GAP: Space = Space(4.0);
/// A boundary chip's padding (vertical, horizontal), in points.
pub(crate) const BOUNDARY_CHIP_PAD_PT: (f32, f32) = (1.0, 5.0);
