//! The Appearance settings' ready-made values (`docs/design.md` §4 and §5,
//! from the mock's Settings page): the background colour presets, the
//! harmony steps that place extra colours, and Shape and type's three
//! starting points.

use crate::design::{Background, BackgroundColour, TYPE_BODY_PT};

use super::Shape;

/// A background colour preset: its colours and strength.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Preset {
    pub(crate) id: PresetId,
    colours: &'static [(f32, f32)],
    strength: f32,
}

/// Which preset, for its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PresetId {
    Neutral,
    Ink,
    Slate,
    Moss,
    Teal,
    Plum,
    Rose,
    Ember,
    Sand,
    Dusk,
    Lagoon,
    Aurora,
    Sunset,
    Forest,
    MonoBlue,
    MonoGreen,
}

/// The presets in the mock's order, `(hue°, vividness)` per colour.
pub(crate) const PRESETS: [Preset; 16] = [
    preset(PresetId::Neutral, &[], 0.0),
    preset(PresetId::Ink, &[(255.0, 0.45)], 0.40),
    preset(PresetId::Slate, &[(235.0, 0.25)], 0.55),
    preset(PresetId::Moss, &[(135.0, 0.55)], 0.45),
    preset(PresetId::Teal, &[(190.0, 0.55)], 0.45),
    preset(PresetId::Plum, &[(320.0, 0.55)], 0.45),
    preset(PresetId::Rose, &[(10.0, 0.50)], 0.40),
    preset(PresetId::Ember, &[(45.0, 0.60)], 0.40),
    preset(PresetId::Sand, &[(80.0, 0.40)], 0.45),
    preset(PresetId::Dusk, &[(290.0, 0.55), (20.0, 0.55)], 0.50),
    preset(PresetId::Lagoon, &[(200.0, 0.6), (150.0, 0.55)], 0.5),
    preset(
        PresetId::Aurora,
        &[(150.0, 0.6), (270.0, 0.6), (200.0, 0.5)],
        0.55,
    ),
    preset(PresetId::Sunset, &[(30.0, 0.65), (350.0, 0.6)], 0.5),
    preset(PresetId::Forest, &[(140.0, 0.5), (100.0, 0.4)], 0.5),
    preset(PresetId::MonoBlue, &[(250.0, 0.85)], 0.7),
    preset(PresetId::MonoGreen, &[(135.0, 0.85)], 0.7),
];

/// How many presets one page of the row shows.
pub(crate) const PRESETS_PER_PAGE: usize = 7;

const fn preset(id: PresetId, colours: &'static [(f32, f32)], strength: f32) -> Preset {
    Preset {
        id,
        colours,
        strength,
    }
}

impl Preset {
    /// Its colours and strength as a background.
    pub(crate) fn background(&self) -> Background {
        let colours: Vec<BackgroundColour> = self
            .colours
            .iter()
            .map(|&(hue_degrees, vividness)| BackgroundColour {
                hue_degrees,
                vividness,
            })
            .collect();
        Background::new(&colours, self.strength)
    }

    /// The preset `background` is, if any: the same colours and strength.
    pub(crate) fn matching(background: &Background) -> Option<Preset> {
        PRESETS.into_iter().find(|preset| {
            let mine = preset.background();
            let same_strength = (mine.strength - background.strength).abs() < 0.005
                || mine.colours().is_empty() && background.colours().is_empty();
            same_strength
                && mine.colours().len() == background.colours().len()
                && mine
                    .colours()
                    .iter()
                    .zip(background.colours())
                    .all(|(a, b)| {
                        (a.hue_degrees - b.hue_degrees).abs() < 0.5
                            && (a.vividness - b.vividness).abs() < 0.005
                    })
        })
    }
}

/// The harmony action's arrangements, in the order it steps through them:
/// each is the extra colours' hue offsets from the first (§4: opposite,
/// three evenly spaced, neighbours, split opposite).
pub(crate) const HARMONIES: [&[f32]; 4] =
    [&[180.0], &[120.0, 240.0], &[30.0, -30.0], &[150.0, 210.0]];

/// The harmony after `step` applied to `background`: the first colour
/// stays, the others take its vividness at the arrangement's offsets.
pub(crate) fn harmony(background: &Background, step: usize) -> Background {
    let Some(first) = background.first() else {
        return *background;
    };
    let offsets = HARMONIES[step % HARMONIES.len()];
    let mut colours = vec![first];
    colours.extend(offsets.iter().map(|offset| BackgroundColour {
        hue_degrees: first.hue_degrees + offset,
        vividness: first.vividness,
    }));
    Background::new(&colours, background.strength)
}

/// Shape and type's starting points (§5: Balanced is Papo's default).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Direction {
    Compact,
    Balanced,
    Airy,
}

impl Direction {
    pub(crate) const ALL: [Direction; 3] =
        [Direction::Compact, Direction::Balanced, Direction::Airy];

    /// Its five values (§5).
    pub(crate) fn shape(self) -> Shape {
        match self {
            Direction::Compact => Shape {
                text_pt: 13.0,
                roundness: 0.75,
                spacing: 0.88,
                blur_pt: 12.0,
                shadow_depth: 0.80,
            },
            Direction::Balanced => Shape {
                text_pt: TYPE_BODY_PT,
                roundness: 1.0,
                spacing: 1.0,
                blur_pt: 16.0,
                shadow_depth: 1.0,
            },
            Direction::Airy => Shape {
                text_pt: 15.0,
                roundness: 1.30,
                spacing: 1.15,
                blur_pt: 22.0,
                shadow_depth: 1.15,
            },
        }
    }

    /// The starting point `shape` is, if it is one.
    pub(crate) fn matching(shape: &Shape) -> Option<Direction> {
        Direction::ALL
            .into_iter()
            .find(|direction| direction.shape().same_as(shape))
    }
}
