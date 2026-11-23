//! Everything a view draws with: the colours (`Palette`) and the sizes at
//! the Appearance settings (`Scale`). One value per frame, built by the
//! shell and handed down; no view reads a setting itself.

use rune::TextStyle;

use super::density::GUTTER;
use super::{
    Density, Font, Lead, Palette, Radius, Space, Spacing, TYPE_BODY_PT, TextMultiple, Theme,
};

/// What the Shape and type settings multiply.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Scale {
    /// `type.body` in points: the Text size setting.
    pub(crate) text_pt: f32,
    /// What every `Space` is multiplied by: the Spacing setting.
    pub(crate) spacing: f32,
    /// What every `Radius` is multiplied by: the Roundness setting.
    pub(crate) roundness: f32,
}

impl Default for Scale {
    fn default() -> Self {
        Scale {
            text_pt: TYPE_BODY_PT,
            spacing: 1.0,
            roundness: 1.0,
        }
    }
}

/// The look of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Look {
    /// The colours.
    pub(crate) palette: Palette,
    scale: Scale,
}

impl Look {
    pub(crate) fn new(theme: Theme) -> Look {
        Look {
            palette: Palette::new(theme),
            scale: Scale::default(),
        }
    }

    /// `font` at the Text size setting: its size and line height times the
    /// setting over the default.
    pub(crate) fn font(&self, font: Font) -> TextStyle {
        let factor = self.scale.text_pt / TYPE_BODY_PT;
        let style = font.0;
        TextStyle {
            size: style.size * factor,
            line_height: style.line_height * factor,
            ..style
        }
    }

    /// `space` in points at the Spacing setting.
    pub(crate) fn space(&self, space: Space) -> f32 {
        space.0 * self.scale.spacing
    }

    /// `radius` in points at the Roundness setting.
    pub(crate) fn radius(&self, radius: Radius) -> f32 {
        radius.0 * self.scale.roundness
    }

    /// `length` in points at the Text size setting.
    pub(crate) fn text_multiple(&self, length: TextMultiple) -> f32 {
        length.0 * self.scale.text_pt
    }

    /// `density`'s distances at the settings: every gap and padding times
    /// Spacing, the Extra compact gutter a multiple of the text size, an
    /// avatar's side unchanged.
    pub(crate) fn spacing(&self, density: Density) -> Spacing {
        let base = density.base_spacing();
        let lead = match base.lead {
            Lead::Avatar(side_pt) => Lead::Avatar(side_pt),
            Lead::Gutter(_) => Lead::Gutter(self.text_multiple(GUTTER)),
        };
        Spacing {
            between_groups_pt: self.space(Space(base.between_groups_pt)),
            within_group_pt: self.space(Space(base.within_group_pt)),
            row_padding_pt: self.space(Space(base.row_padding_pt)),
            lead,
            lead_gap_pt: self.space(Space(base.lead_gap_pt)),
        }
    }
}
