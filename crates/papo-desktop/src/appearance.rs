//! The Appearance settings (`docs/design.md` §4 to §6 and §8): the theme,
//! the background colour and grain, the accent, message spacing and
//! grouping, Shape and type's five values, the system font and the Glass
//! switch. Every one is this device's, one value for every account: the
//! account's `theme`, `fontSize` and `messageDensity` cannot hold them
//! (B-016 to B-018), so none follows the account until the backend can,
//! and while nothing syncs message spacing is not kept per account
//! either (Christian, 2026-11-23).
//!
//! Owns the values, their defaults and their place in this device's
//! settings file (`rune::prefs`). A change is saved before it is kept, so
//! what the settings show is what is saved (§8, Saving). The tokens drawn
//! from them are `design`'s (`Look`).

use rune::prefs::Prefs;

use crate::design::{Accent, Background, MessageLayout, TYPE_BODY_PT, Theme};
use crate::device_settings::{SettingsFile, Stored, Write};

pub(crate) mod presets;
mod stored;

/// Shape and type's five values (§5).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Shape {
    /// `type.body`, in points.
    pub(crate) text_pt: f32,
    /// What every radius is multiplied by.
    pub(crate) roundness: f32,
    /// What every space is multiplied by.
    pub(crate) spacing: f32,
    /// How far glass blurs what is behind it, in points.
    pub(crate) blur_pt: f32,
    /// What every shadow is multiplied by.
    pub(crate) shadow_depth: f32,
}

/// One of Shape and type's values, for its slider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum ShapeValue {
    TextSize,
    Roundness,
    Spacing,
    Blur,
    ShadowDepth,
}

impl ShapeValue {
    pub(crate) const ALL: [ShapeValue; 5] = [
        ShapeValue::TextSize,
        ShapeValue::Roundness,
        ShapeValue::Spacing,
        ShapeValue::Blur,
        ShapeValue::ShadowDepth,
    ];

    /// The range the slider covers (§5).
    pub(crate) fn range(self) -> (f32, f32) {
        match self {
            ShapeValue::TextSize => (TEXT_SIZE_MIN_PT, TEXT_SIZE_MAX_PT),
            ShapeValue::Roundness => (0.0, 1.6),
            ShapeValue::Spacing => (0.8, 1.25),
            ShapeValue::Blur => (0.0, 32.0),
            ShapeValue::ShadowDepth => (0.0, 1.5),
        }
    }
}

/// The Glass setting at first: frosted, half way to liquid.
pub(crate) const DEFAULT_GLASS: f32 = 0.5;

/// The smallest and largest text size, in points (§5).
pub(crate) const TEXT_SIZE_MIN_PT: f32 = 12.0;
pub(crate) const TEXT_SIZE_MAX_PT: f32 = 16.0;
/// How far Ctrl+= and Ctrl+- move the text size, in points.
pub(crate) const TEXT_SIZE_STEP_PT: f32 = 1.0;

impl Shape {
    pub(crate) fn get(&self, value: ShapeValue) -> f32 {
        match value {
            ShapeValue::TextSize => self.text_pt,
            ShapeValue::Roundness => self.roundness,
            ShapeValue::Spacing => self.spacing,
            ShapeValue::Blur => self.blur_pt,
            ShapeValue::ShadowDepth => self.shadow_depth,
        }
    }

    /// The same shape with `value` at `to`, held to its range.
    pub(crate) fn with(mut self, value: ShapeValue, to: f32) -> Shape {
        let (min, max) = value.range();
        let to = to.clamp(min, max);
        match value {
            ShapeValue::TextSize => self.text_pt = to,
            ShapeValue::Roundness => self.roundness = to,
            ShapeValue::Spacing => self.spacing = to,
            ShapeValue::Blur => self.blur_pt = to,
            ShapeValue::ShadowDepth => self.shadow_depth = to,
        }
        self
    }
}

impl Shape {
    /// Whether every value equals `other`'s, to the slider's resolution.
    pub(crate) fn same_as(&self, other: &Shape) -> bool {
        ShapeValue::ALL
            .into_iter()
            .all(|value| (self.get(value) - other.get(value)).abs() < 0.005)
    }
}

impl Default for Shape {
    /// The Balanced direction, Papo's defaults (§5).
    fn default() -> Shape {
        Shape {
            text_pt: TYPE_BODY_PT,
            roundness: 1.0,
            spacing: 1.0,
            blur_pt: 16.0,
            shadow_depth: 1.0,
        }
    }
}

/// Everything Appearance holds for the whole device.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Appearance {
    /// The chosen theme; none until the person picks one, which follows
    /// the system (Graphite or Light).
    pub(crate) theme: Option<Theme>,
    pub(crate) background: Background,
    /// A fine noise over the surfaces (0–1).
    pub(crate) grain: f32,
    pub(crate) accent: Accent,
    /// Message spacing, Group and Stack (§6).
    pub(crate) messages: MessageLayout,
    pub(crate) shape: Shape,
    /// The system's interface font instead of Inter.
    pub(crate) system_font: bool,
    /// How glassy the floating surfaces are, 0–1: 0 solid, then frosted,
    /// towards 1 clear liquid glass (`docs/design.md` §5; Christian,
    /// 2026-11-23). One value for every theme: the conversation scrolls
    /// under the pills, so OLED's glass shows it too.
    pub(crate) glass: f32,
}

impl Default for Appearance {
    fn default() -> Appearance {
        Appearance {
            theme: None,
            background: Background::default(),
            grain: 0.0,
            accent: Accent::default(),
            messages: MessageLayout::default(),
            shape: Shape::default(),
            system_font: false,
            glass: DEFAULT_GLASS,
        }
    }
}

impl Appearance {
    /// The theme drawn: the chosen one, else the system's.
    pub(crate) fn theme_or(&self, system_dark: bool) -> Theme {
        self.theme
            .unwrap_or_else(|| Theme::following_system(system_dark))
    }
}

/// The Appearance settings and where they are saved. A change shows at
/// once and is written off the UI thread, one write at a time; a failed
/// write takes the settings back to what the file holds.
pub(crate) struct AppearanceStore {
    stored: Stored<Appearance>,
    file: SettingsFile,
}

impl Default for AppearanceStore {
    /// The defaults, kept nowhere: before the settings file is read, and in
    /// tests.
    fn default() -> AppearanceStore {
        AppearanceStore::read(&Prefs::memory())
    }
}

impl AppearanceStore {
    /// The settings in `prefs`, this device's settings file; a missing or
    /// unreadable value takes its default.
    pub(crate) fn read(prefs: &Prefs) -> AppearanceStore {
        AppearanceStore {
            stored: Stored::new(stored::read(prefs)),
            file: SettingsFile::of(prefs),
        }
    }

    /// The settings drawn.
    pub(crate) fn appearance(&self) -> &Appearance {
        self.stored.shown()
    }

    /// Shows `appearance` at once; the settings to write now, unless a
    /// write is running (they then follow it) or nothing changed.
    pub(crate) fn change(&mut self, appearance: Appearance) -> Option<Appearance> {
        self.stored.change(appearance)
    }

    /// The write of `appearance` landed (`ok`) or did not, which takes the
    /// settings back to what the file holds; the settings to write next.
    pub(crate) fn written(&mut self, appearance: Appearance, ok: bool) -> Option<Appearance> {
        self.stored.written(appearance, ok)
    }

    /// The write of the change waiting behind the running one, if any.
    pub(crate) fn pending_write(&self) -> Option<Write> {
        self.stored
            .pending()
            .map(|appearance| self.write_of(appearance))
    }

    /// The write that saves `appearance`.
    pub(crate) fn write_of(&self, appearance: &Appearance) -> Write {
        self.file.write(vec![stored::entry(appearance)])
    }
}

/// The text size one step from `text_pt` (`up` larger), held to its range,
/// on whole points: Ctrl+= and Ctrl+- (§12).
pub(crate) fn text_size_step(text_pt: f32, up: bool) -> f32 {
    let next = if up {
        (text_pt + TEXT_SIZE_STEP_PT).floor()
    } else {
        (text_pt - TEXT_SIZE_STEP_PT).ceil()
    };
    next.clamp(TEXT_SIZE_MIN_PT, TEXT_SIZE_MAX_PT)
}

/// Ctrl+0's text size: the default.
pub(crate) const TEXT_SIZE_DEFAULT_PT: f32 = TYPE_BODY_PT;

#[cfg(test)]
mod tests;
