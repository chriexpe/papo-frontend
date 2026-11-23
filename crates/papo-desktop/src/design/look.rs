//! Everything a view draws with: the colours (`Palette`) and the sizes at
//! the Appearance settings (`Scale`). One value per frame, built by the
//! shell and handed down; no view reads a setting itself.

use rune::{Color, El, Family, TextStyle};

use super::density::GUTTER;
use super::{
    Accent, Background, CHROME_GRADIENT_DEGREES, ChromeFill, Density, Font, Lead, PILL_OUTLINE_PT,
    Palette, Radius, Space, Spacing, TYPE_BODY_PT, TextMultiple, Theme,
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
    /// The Glass switch for this theme, unless the system reduces
    /// transparency, which makes every theme solid.
    pub(crate) on: bool,
    /// How far glass blurs what is behind it, in points.
    pub(crate) blur_pt: f32,
    /// What the shadows are multiplied by; 0 removes them.
    pub(crate) shadow_depth: f32,
}

impl Default for Glass {
    /// Solid at the default depth: what tests and the first frame draw.
    fn default() -> Glass {
        Glass {
            on: false,
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

/// The opacity of a glass pill's tint over what it blurs, per theme in
/// `Theme::ALL`'s order (the mock's values).
const GLASS_TINT_ALPHA: [f32; 4] = [0.82, 0.74, 0.80, 0.84];
/// The ink of a light theme's shadows (§5): `rgb(25,30,45)`, in Rune's
/// linear light.
const LIGHT_SHADOW_INK: Color = Color {
    r: 0.009_721_217,
    g: 0.012_983_032,
    b: 0.026_241_222,
    a: 1.0,
};
/// The soft lift: offset and blur in points at depth ×1, and its opacity on
/// OLED, the other dark themes and light themes.
const SHADOW_LIFT: (f32, f32) = (7.0, 24.0);
const SHADOW_LIFT_ALPHA: (f32, f32, f32) = (0.60, 0.40, 0.10);
/// The tight contact shadow, likewise, on dark and light themes.
const SHADOW_CONTACT: (f32, f32) = (1.0, 3.0);
const SHADOW_CONTACT_ALPHA: (f32, f32) = (0.30, 0.08);
/// The depth past which a shadow's opacity stops growing (the mock's cap).
const SHADOW_ALPHA_DEPTH_CAP: f32 = 1.3;

/// The look of one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Look {
    /// The colours.
    pub(crate) palette: Palette,
    theme: Theme,
    scale: Scale,
    glass: Glass,
    font_family: Option<&'static str>,
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

    pub(crate) fn with(spec: &LookSpec) -> Look {
        Look {
            palette: Palette::with(spec.theme, spec.accent, &spec.background),
            theme: spec.theme,
            scale: spec.scale,
            glass: spec.glass,
            font_family: spec.font_family,
        }
    }

    pub(crate) fn theme(&self) -> Theme {
        self.theme
    }

    /// A floating pill's surface on `el` (§5): glass (the pill colour,
    /// translucent, over a blur of what is behind it) or solid, its
    /// outline, and the lift and contact shadows at the depth setting.
    fn pill_surface(&self, el: El) -> El {
        let palette = &self.palette;
        let el = el.border(PILL_OUTLINE_PT, palette.line_pill);
        let el = if self.glass.on {
            let index = Theme::ALL
                .iter()
                .position(|theme| *theme == self.theme)
                .unwrap_or_default();
            el.backdrop_blur(self.glass.blur_pt)
                .fill(palette.surface_pill.alpha(GLASS_TINT_ALPHA[index]))
        } else {
            el.fill(palette.surface_pill)
        };
        self.shadows(el)
    }

    /// The lift and contact shadows at the depth setting; none at 0.
    fn shadows(&self, el: El) -> El {
        let depth = self.glass.shadow_depth;
        if depth <= 0.001 {
            return el;
        }
        let dark = self.theme.is_dark();
        let ink = if dark { Color::BLACK } else { LIGHT_SHADOW_INK };
        let alpha = depth.min(SHADOW_ALPHA_DEPTH_CAP);
        let lift = match self.theme {
            Theme::Oled => SHADOW_LIFT_ALPHA.0,
            _ if dark => SHADOW_LIFT_ALPHA.1,
            _ => SHADOW_LIFT_ALPHA.2,
        };
        let contact = if dark {
            SHADOW_CONTACT_ALPHA.0
        } else {
            SHADOW_CONTACT_ALPHA.1
        };
        el.shadow(
            SHADOW_LIFT.0 * depth,
            SHADOW_LIFT.1 * depth,
            ink.alpha(lift * alpha),
        )
        .shadow(
            SHADOW_CONTACT.0,
            SHADOW_CONTACT.1 * depth,
            ink.alpha(contact * alpha),
        )
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

/// The surfaces a view puts under its elements, as builder steps.
pub(crate) trait Surfaces {
    /// A floating pill's surface: glass or solid, outline and shadows.
    fn pill_surface(self, look: &Look) -> Self;
    /// A chrome surface (the rail, a sidebar): its colour, or the
    /// background colours' gradient across it.
    fn chrome_fill(self, fill: &ChromeFill) -> Self;
}

impl Surfaces for El {
    fn pill_surface(self, look: &Look) -> El {
        look.pill_surface(self)
    }

    fn chrome_fill(self, fill: &ChromeFill) -> El {
        let el = self.fill(fill.first());
        match fill.gradient() {
            Some(stops) => el.linear_gradient(CHROME_GRADIENT_DEGREES, stops),
            None => el,
        }
    }
}
