//! Message density (`docs/design.md` §5 and §6): the four spacings and the
//! two grouping switches. Owns the numbers only; the timeline lays rows out
//! with them, and the Appearance settings (D6) will choose them.

use super::{SPACE_SM_PT, TYPE_BODY};

/// How tightly messages sit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the Appearance settings (D6) choose a density; the spacing tests use each now"
    )
)]
pub(crate) enum Density {
    Compact,
    #[default]
    Normal,
    Comfortable,
    /// No avatar: the author's name only, the time in a narrow gutter.
    ExtraCompact,
}

/// The density and the two grouping switches, as one value a row's memo
/// can depend on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MessageLayout {
    pub(crate) density: Density,
    /// Consecutive messages from one author hide the repeated header.
    pub(crate) grouped: bool,
    /// Extra compact only: the name sits on its own line above the text.
    pub(crate) stacked: bool,
}

impl Default for MessageLayout {
    fn default() -> Self {
        MessageLayout {
            density: Density::Normal,
            grouped: true,
            stacked: true,
        }
    }
}

/// What leads a message's row, before its content column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Lead {
    /// An avatar this many points square.
    Avatar(f32),
    /// A time gutter this many points wide.
    Gutter(f32),
}

/// One density's distances, in points at spacing ×1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Spacing {
    /// From one group (or a date separator) to the next.
    pub(crate) between_groups_pt: f32,
    /// From one message to the next inside a group.
    pub(crate) within_group_pt: f32,
    /// A row's padding above and below its content.
    pub(crate) row_padding_pt: f32,
    pub(crate) lead: Lead,
    /// From the lead column to the content column.
    pub(crate) lead_gap_pt: f32,
}

/// A row's padding left and right of its content, in points.
pub(crate) const MESSAGE_ROW_INSET_PT: f32 = 8.0;
/// Between a message's blocks (header, text), at every density.
pub(crate) const MESSAGE_BLOCK_GAP_PT: f32 = SPACE_SM_PT;
/// Between the name and the text of an inline Extra compact row.
pub(crate) const MESSAGE_NAME_GAP_PT: f32 = 8.0;
/// Between the name and the time in a message's header.
pub(crate) const MESSAGE_HEADER_GAP_PT: f32 = 6.0;
/// How far a message's avatar sits below the row's padding, as a multiple
/// of the text size: the bundled Inter 4.1's distance from the last
/// baseline to the bottom of its line box at line height 1.45, so a row's
/// background shows as much above the avatar as below the last line.
const AVATAR_TOP_TEXT_SIZES: f32 = 0.3786;
/// How far a message's avatar sits below the row's padding, in points.
pub(crate) const AVATAR_TOP_PT: f32 = AVATAR_TOP_TEXT_SIZES * TYPE_BODY.size;
/// The Extra compact time gutter, as a multiple of the text size.
const GUTTER_TEXT_SIZES: f32 = 3.2;

impl Density {
    /// Its distances (`docs/design.md` §5).
    pub(crate) fn spacing(self) -> Spacing {
        match self {
            Density::Compact => Spacing {
                between_groups_pt: 8.0,
                within_group_pt: 0.0,
                row_padding_pt: 1.0,
                lead: Lead::Avatar(28.0),
                lead_gap_pt: 10.0,
            },
            Density::Normal => Spacing {
                between_groups_pt: 14.0,
                within_group_pt: 2.0,
                row_padding_pt: 2.0,
                lead: Lead::Avatar(34.0),
                lead_gap_pt: 10.0,
            },
            Density::Comfortable => Spacing {
                between_groups_pt: 20.0,
                within_group_pt: 4.0,
                row_padding_pt: 3.0,
                lead: Lead::Avatar(40.0),
                lead_gap_pt: 12.0,
            },
            Density::ExtraCompact => Spacing {
                between_groups_pt: 6.0,
                within_group_pt: 0.0,
                row_padding_pt: 1.0,
                lead: Lead::Gutter(GUTTER_TEXT_SIZES * TYPE_BODY.size),
                lead_gap_pt: 8.0,
            },
        }
    }
}
