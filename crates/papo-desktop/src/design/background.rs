//! The user's colours (`docs/design.md` §4, Accent and Background colour):
//! the accent hue, up to three background colours with their strength, and
//! how the first colour tints every neutral token. Owns the colour math
//! only; the Appearance settings store the values.

use rune::{Color, oklch};

/// The accent hues of §4.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Accent {
    #[default]
    Blue,
    Indigo,
    Teal,
    Green,
    Pink,
    Orange,
}

impl Accent {
    /// Every accent, in the settings' order.
    pub(crate) const ALL: [Accent; 6] = [
        Accent::Blue,
        Accent::Indigo,
        Accent::Teal,
        Accent::Green,
        Accent::Pink,
        Accent::Orange,
    ];

    /// Its OKLCH hue in degrees.
    pub(crate) fn hue_degrees(self) -> f32 {
        match self {
            Accent::Blue => 256.0,
            Accent::Indigo => 278.0,
            Accent::Teal => 196.0,
            Accent::Green => 152.0,
            Accent::Pink => 352.0,
            Accent::Orange => 52.0,
        }
    }
}

/// One background colour: a hue and how vivid it is (0–1), a point on the
/// settings' colour field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BackgroundColour {
    pub(crate) hue_degrees: f32,
    pub(crate) vividness: f32,
}

/// The most background colours: the first tints every surface, the others
/// make the chrome's gradient.
pub(crate) const MAX_BACKGROUND_COLOURS: usize = 3;

/// The background colours and their strength.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Background {
    colours: [BackgroundColour; MAX_BACKGROUND_COLOURS],
    count: usize,
    /// How much of the colour reaches the surfaces (0–1).
    pub(crate) strength: f32,
}

impl Default for Background {
    /// §4's default: one colour, 255° at vividness 0.45, strength 0.40.
    fn default() -> Background {
        Background::new(
            &[BackgroundColour {
                hue_degrees: 255.0,
                vividness: 0.45,
            }],
            0.40,
        )
    }
}

/// The tint's chroma at full strength and vividness, before the
/// mid-tone weighting (§4).
const TINT_CHROMA: f32 = 0.075;
/// How close to black or white, in OKLCH lightness, the tint fades out, so
/// pure black and pure white stay untinted.
const TINT_EDGE: f32 = 0.03;

impl Background {
    /// The first `MAX_BACKGROUND_COLOURS` of `colours` at `strength`, each
    /// value clamped to its range.
    pub(crate) fn new(colours: &[BackgroundColour], strength: f32) -> Background {
        let mut kept = [BackgroundColour {
            hue_degrees: 0.0,
            vividness: 0.0,
        }; MAX_BACKGROUND_COLOURS];
        let count = colours.len().min(MAX_BACKGROUND_COLOURS);
        for (slot, colour) in kept.iter_mut().zip(colours) {
            *slot = BackgroundColour {
                hue_degrees: colour.hue_degrees.rem_euclid(360.0),
                vividness: colour.vividness.clamp(0.0, 1.0),
            };
        }
        Background {
            colours: kept,
            count,
            strength: strength.clamp(0.0, 1.0),
        }
    }

    /// The colours, first to last.
    pub(crate) fn colours(&self) -> &[BackgroundColour] {
        &self.colours[..self.count]
    }

    /// The tint `colour` gives a neutral token at OKLCH `lightness`, as
    /// (chroma, hue in degrees): chroma `strength × vividness × 0.075 ×
    /// (0.3 + 0.7 × (1 − |2L − 1|))`, strongest in the mid-tones, fading to
    /// none over the last 0.03 of L at both ends. Text takes half of it.
    /// No colour, no tint.
    pub(crate) fn tint(
        &self,
        colour: Option<BackgroundColour>,
        lightness: f32,
        text: bool,
    ) -> (f32, f32) {
        let Some(colour) = colour else {
            return (0.0, 0.0);
        };
        let midtone = 0.3 + 0.7 * (1.0 - (2.0 * lightness - 1.0).abs());
        let edge = (lightness.min(1.0 - lightness) / TINT_EDGE).clamp(0.0, 1.0);
        let mut chroma = self.strength * colour.vividness * TINT_CHROMA * midtone * edge;
        if text {
            chroma *= 0.5;
        }
        (chroma, colour.hue_degrees)
    }

    /// The first colour, which tints every surface.
    pub(crate) fn first(&self) -> Option<BackgroundColour> {
        self.colours().first().copied()
    }

    /// A neutral token at `lightness`, tinted by the first colour as the
    /// formula gives it (lines and other colours no text sits on).
    pub(crate) fn neutral(&self, lightness: f32, text: bool) -> Color {
        let (chroma, hue_degrees) = self.tint(self.first(), lightness, text);
        oklch(lightness, chroma, hue_degrees)
    }
}

/// A chrome surface's fill: one colour, or with two or three background
/// colours a gradient across it at §4's 165°.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ChromeFill {
    stops: [Color; MAX_BACKGROUND_COLOURS],
    count: usize,
}

/// The chrome gradient's angle, in degrees (§4).
pub(crate) const CHROME_GRADIENT_DEGREES: f32 = 165.0;

impl ChromeFill {
    /// The fill: one stop per background colour, each `surface` of it (the
    /// first colour's, or the untinted one with none).
    pub(super) fn new(
        background: &Background,
        surface: impl Fn(Option<BackgroundColour>) -> Color,
    ) -> ChromeFill {
        let colours = background.colours();
        let mut stops = [surface(background.first()); MAX_BACKGROUND_COLOURS];
        for (stop, colour) in stops.iter_mut().zip(colours) {
            *stop = surface(Some(*colour));
        }
        ChromeFill {
            stops,
            count: colours.len().max(1),
        }
    }

    /// Its colour when it is one colour; its first stop otherwise, which is
    /// also what `surface.*` names for contrast.
    pub(crate) fn first(&self) -> Color {
        self.stops[0]
    }

    /// Its stops, `(position, colour)`, when it is a gradient.
    #[expect(clippy::cast_precision_loss, reason = "at most three stops")]
    pub(crate) fn gradient(&self) -> Option<Vec<(f32, Color)>> {
        if self.count < 2 {
            return None;
        }
        let last = (self.count - 1) as f32;
        Some(
            self.stops[..self.count]
                .iter()
                .enumerate()
                .map(|(index, colour)| (index as f32 / last, *colour))
                .collect(),
        )
    }

    /// Every colour it draws, for the contrast tests.
    #[cfg(test)]
    pub(crate) fn colours(&self) -> &[Color] {
        &self.stops[..self.count]
    }
}
