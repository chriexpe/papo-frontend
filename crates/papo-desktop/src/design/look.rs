//! Everything a view draws with: the colours (`Palette`) and the sizes at
//! the Appearance settings (`Scale`). One value per frame, built by the
//! shell and handed down; no view reads a setting itself. The surfaces
//! built from it (glass, shadows) are `surfaces`'.

use std::hash::{DefaultHasher, Hash, Hasher};

use rune::{Family, TextStyle};

use super::density::GUTTER;
use super::{
    Accent, Background, Density, Font, Lead, Palette, Radius, Space, Spacing, TYPE_BODY_PT,
    TextMultiple, Theme,
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

/// How floating pills draw (§5, Glass and shadow).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Glass {
    /// How glassy, 0–1: 0 solid, then frosted, towards 1 clear liquid
    /// glass (the window's compositor, `glass`, draws the lens).
    pub(crate) glassy: f32,
    /// How far glass blurs what is behind it, in points.
    pub(crate) blur_pt: f32,
    /// What the shadows are multiplied by; 0 removes them.
    pub(crate) shadow_depth: f32,
}

impl Default for Glass {
    /// Solid at the default depth: what tests and the first frame draw.
    fn default() -> Glass {
        Glass {
            glassy: 0.0,
            blur_pt: 16.0,
            shadow_depth: 1.0,
        }
    }
}

/// What a frame's look is made from: the Appearance settings, the system's
/// preferences applied.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LookSpec {
    pub(crate) theme: Theme,
    pub(crate) accent: Accent,
    pub(crate) background: Background,
    pub(crate) scale: Scale,
    pub(crate) glass: Glass,
    /// The system's interface font family instead of Inter, when the
    /// setting is on and the system names one.
    pub(crate) font_family: Option<&'static str>,
}

/// The look of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Look {
    /// The colours.
    pub(crate) palette: Palette,
    pub(super) theme: Theme,
    scale: Scale,
    pub(super) glass: Glass,
    font_family: Option<&'static str>,
    /// Everything the look is made from, as one number (`LookSpec::key`).
    key: u64,
}

/// A look hashes as what it is made from, so a memoised view that puts it
/// in its version rebuilds whenever a setting it is drawn with changes.
impl Hash for Look {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

impl LookSpec {
    /// Every setting the look is made from, as one number; floats by their
    /// bits.
    fn key(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.theme.hash(&mut hasher);
        self.accent.hash(&mut hasher);
        for colour in self.background.colours() {
            colour.hue_degrees.to_bits().hash(&mut hasher);
            colour.vividness.to_bits().hash(&mut hasher);
        }
        self.background.colours().len().hash(&mut hasher);
        let Scale {
            text_pt,
            spacing,
            roundness,
        } = self.scale;
        let Glass {
            glassy,
            blur_pt,
            shadow_depth,
        } = self.glass;
        for value in [
            self.background.strength,
            text_pt,
            spacing,
            roundness,
            glassy,
            blur_pt,
            shadow_depth,
        ] {
            value.to_bits().hash(&mut hasher);
        }
        self.font_family.hash(&mut hasher);
        hasher.finish()
    }
}

impl Look {
    /// `theme` at the defaults: what tests draw.
    #[cfg(test)]
    pub(crate) fn new(theme: Theme) -> Look {
        Look::with(&LookSpec {
            theme,
            accent: Accent::default(),
            background: Background::default(),
            scale: Scale::default(),
            glass: Glass::default(),
            font_family: None,
        })
    }

    /// `theme` at the defaults but for `scale`: what the spacing tests
    /// draw at the smallest and largest text size and spacing.
    #[cfg(test)]
    pub(crate) fn scaled(theme: Theme, scale: Scale) -> Look {
        Look::with(&LookSpec {
            theme,
            accent: Accent::default(),
            background: Background::default(),
            scale,
            glass: Glass::default(),
            font_family: None,
        })
    }

    pub(crate) fn with(spec: &LookSpec) -> Look {
        Look {
            palette: Palette::with(spec.theme, spec.accent, &spec.background),
            theme: spec.theme,
            scale: spec.scale,
            glass: spec.glass,
            font_family: spec.font_family,
            key: spec.key(),
        }
    }

    pub(crate) fn theme(&self) -> Theme {
        self.theme
    }

    /// `font` at the Text size setting: its size and line height times the
    /// setting over the default.
    pub(crate) fn font(&self, font: Font) -> TextStyle {
        let factor = self.scale.text_pt / TYPE_BODY_PT;
        let style = font.0;
        let family = match (self.font_family, style.family) {
            // Only the sans slot follows "Use the system font"; code stays
            // mono.
            (Some(name), Family::Sans) => Family::Named(name),
            (_, family) => family,
        };
        TextStyle {
            size: style.size * factor,
            line_height: style.line_height * factor,
            family,
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

    /// The radius, in points, of a rounded surface whose edges sit `gap_pt`
    /// inside a surface of radius `outer_pt` (§5, Nested corners):
    /// concentric with it, so the two curves stay parallel; square once the
    /// gap reaches the outer radius. `outer_pt` is the outer surface's drawn
    /// radius (`Look::radius` of its token, or half its height for a capsule).
    pub(crate) fn nested_radius(outer_pt: f32, gap_pt: f32) -> f32 {
        (outer_pt - gap_pt).max(0.0)
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

#[cfg(test)]
mod tests {
    #[test]
    fn a_nested_corner_is_concentric_and_never_below_square() {
        assert!((super::Look::nested_radius(14.0, 4.0) - 10.0).abs() < f32::EPSILON);
        assert!(super::Look::nested_radius(8.0, 12.0).abs() < f32::EPSILON);
    }
}
