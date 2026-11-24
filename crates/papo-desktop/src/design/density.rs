//! Message density (`docs/design.md` §5, §6 and §8): the four spacings,
//! the two grouping switches, and whether rows show their time and avatar.
//! Owns the numbers at spacing ×1 and the default text size;
//! `Look::row_spacing` scales them for a layout, and the timeline lays rows
//! out with that.

use super::{SPACE_SM, Space, TextMultiple};

/// How tightly messages sit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Density {
    Compact,
    #[default]
    Normal,
    Comfortable,
    /// No avatar: the author's name only, the time in a narrow gutter.
    ExtraCompact,
}

/// How rows are laid out, as one value a row's memo can depend on: the
/// density and grouping switches (Appearance's), and whether rows show
/// their time and avatar (Conversations'; the shell joins the two,
/// `Shell::message_layout`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is one of the settings' independent switches"
)]
pub(crate) struct MessageLayout {
    pub(crate) density: Density,
    /// Consecutive messages from one author hide the repeated header.
    pub(crate) grouped: bool,
    /// Without avatars only: the name sits on its own line above the text.
    pub(crate) stacked: bool,
    /// "Show times": off, no row draws its time.
    pub(crate) times: bool,
    /// "Show avatars": off, every density draws Extra compact's shapes at
    /// its own spacing.
    pub(crate) avatars: bool,
}

impl Default for MessageLayout {
    fn default() -> Self {
        MessageLayout {
            density: Density::Normal,
            grouped: true,
            stacked: true,
            times: true,
            avatars: true,
        }
    }
}

impl MessageLayout {
    /// Whether rows draw avatars: not at Extra compact, nor with Show
    /// avatars off. Without them, Stack messages applies.
    pub(crate) fn has_avatars(self) -> bool {
        self.avatars && self.density != Density::ExtraCompact
    }
}

/// What leads a message's row, before its content column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Lead {
    /// An avatar this many points square.
    Avatar(f32),
    /// A time gutter this many points wide.
    Gutter(f32),
    /// Nothing: no avatars and no times, the name at the row's start.
    None,
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

/// A row's padding left and right of its content.
pub(crate) const MESSAGE_ROW_INSET: Space = Space(8.0);
/// Between a message's blocks (header, text), at every density.
pub(crate) const MESSAGE_BLOCK_GAP: Space = SPACE_SM;
/// Between the name and the text of an inline Extra compact row.
pub(crate) const MESSAGE_NAME_GAP: Space = Space(8.0);
/// Between the name and the time in a message's header.
pub(crate) const MESSAGE_HEADER_GAP: Space = Space(6.0);
/// How far a message's avatar sits below the row's padding, as a multiple
/// of the text size: the bundled Inter 4.1's distance from the last
/// baseline to the bottom of its line box at line height 1.45, so a row's
/// background shows as much above the avatar as below the last line.
pub(crate) const AVATAR_TOP: TextMultiple = TextMultiple(0.3786);
/// The Extra compact time gutter, as a multiple of the text size.
pub(super) const GUTTER: TextMultiple = TextMultiple(3.2);

impl Density {
    /// Every density, in the settings' order.
    pub(crate) const ALL: [Density; 4] = [
        Density::Compact,
        Density::Normal,
        Density::Comfortable,
        Density::ExtraCompact,
    ];

    /// Its distances at spacing ×1 and the default text size
    /// (`docs/design.md` §5); `Look::spacing` gives them at the settings.
    pub(super) fn base_spacing(self) -> Spacing {
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
                lead: Lead::Gutter(GUTTER.0 * super::TYPE_BODY_PT),
                lead_gap_pt: 8.0,
            },
        }
    }
}
