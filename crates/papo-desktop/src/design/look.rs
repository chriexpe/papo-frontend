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
    /// How glassy, 0–1: 0 solid, then frosted, towards 1 clear liquid
    /// glass (the window's compositor, `glass`, draws the lens). 0 while
    /// the system reduces transparency.
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

/// The effect mark of a glass surface, which the window's compositor
/// draws as a lens (`glass`).
pub(crate) const GLASS_MARK: u32 = 0x9a55;

/// What a glass surface tells the compositor, in its four effect floats.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GlassData {
    /// The Glass setting, above 0.
    pub(crate) glassy: f32,
    /// Glass blur, in points.
    pub(crate) blur_pt: f32,
    /// The surface's own colour, which tints the lens.
    pub(crate) tint: Color,
}

impl GlassData {
    /// Packed for `El::effect_data`: glassiness, blur, and the tint as its
    /// sRGB `0xRRGGBB` word, carried bit for bit.
    pub(crate) fn pack(self) -> [f32; 4] {
        // `Color::hex` is the sRGB `#rrggbbaa`; its colour part is the word.
        let word = u32::from_str_radix(&self.tint.hex()[1..7], 16).unwrap_or_default();
        [self.glassy, self.blur_pt, f32::from_bits(word), 0.0]
    }

    /// The glassiness, the blur and the tint's sRGB word, as packed.
    pub(crate) fn unpack(data: [f32; 4]) -> (f32, f32, u32) {
        (data[0], data[1], data[2].to_bits())
    }
}
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

    /// `theme` at the defaults but for `scale`: what the spacing tests
    /// draw at the smallest and largest text size and spacing.
    #[cfg(test)]
    pub(crate) fn scaled(theme: Theme, scale: Scale) -> Look {
        Look {
            scale,
            ..Look::new(theme)
        }
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
        self.floating_surface(el, self.palette.surface_pill)
    }

    /// The settings panel's surface (§8): as a pill's, in the panel's own
    /// colour (the sidebar's on dark themes).
    fn panel_surface(&self, el: El) -> El {
        self.floating_surface(el, self.palette.surface_panel)
    }

    /// A floating surface of `colour`: glass or solid, outline, shadows.
    fn floating_surface(&self, el: El, colour: Color) -> El {
        let palette = &self.palette;
        let el = if self.glass.glassy > 0.0 {
            // The compositor draws the lens here, from a copy of what is
            // behind, tinted by the surface's colour.
            let data = GlassData {
                glassy: self.glass.glassy,
                blur_pt: self.glass.blur_pt,
                tint: colour,
            };
            // The outline is drawn after the lens (an outline, inset by
            // its width, where a solid surface has its border), so the
            // edge is the same at every Glass value.
            el.effect(GLASS_MARK)
                .effect_backdrop(true)
                .effect_data(data.pack())
                .outline(PILL_OUTLINE_PT, -PILL_OUTLINE_PT, palette.line_pill)
        } else {
            el.fill(colour).border(PILL_OUTLINE_PT, palette.line_pill)
        };
        // A floating surface covers what is under it: Rune lights a hover
        // group (a message row) whose box holds the pointer unless another
        // group is over it there, so the surface is one, and the rows the
        // conversation scrolls under it don't light through it.
        self.shadows(el.hover_group())
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
    /// The settings panel's surface: a pill's, in the panel colour.
    fn panel_surface(self, look: &Look) -> Self;
    /// A chrome surface (the rail, a sidebar): its colour, or the
    /// background colours' gradient across it.
    fn chrome_fill(self, fill: &ChromeFill) -> Self;
}

impl Surfaces for El {
    fn pill_surface(self, look: &Look) -> El {
        look.pill_surface(self)
    }

    fn panel_surface(self, look: &Look) -> El {
        look.panel_surface(self)
    }

    fn chrome_fill(self, fill: &ChromeFill) -> El {
        let el = self.fill(fill.first());
        match fill.gradient() {
            Some(stops) => el.linear_gradient(CHROME_GRADIENT_DEGREES, stops),
            None => el,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::GlassData;

    #[test]
    fn a_glass_slot_carries_its_settings_and_its_tint_to_the_byte() {
        let data = GlassData {
            glassy: 0.5,
            blur_pt: 16.0,
            tint: rune::Color {
                r: 0.0,
                g: 0.5,
                b: 1.0,
                a: 1.0,
            },
        }
        .pack();
        let (glassy, blur_pt, word) = GlassData::unpack(data);
        assert_eq!((glassy, blur_pt), (0.5, 16.0));
        // Linear 0.5 is sRGB 0xbc.
        assert_eq!(word, 0x0000_bcff);
    }
}
