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

use crate::design::{
    Accent, Background, BackgroundColour, Density, MessageLayout, TYPE_BODY_PT, Theme,
};
use crate::window::APP_ID;

pub(crate) mod presets;

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

/// The settings file's keys.
mod key {
    pub(super) const THEME: &str = "appearance.theme";
    pub(super) const BACKGROUND: &str = "appearance.background";
    pub(super) const STRENGTH: &str = "appearance.strength";
    pub(super) const GRAIN: &str = "appearance.grain";
    pub(super) const ACCENT: &str = "appearance.accent";
    pub(super) const SHAPE: &str = "appearance.shape";
    pub(super) const SYSTEM_FONT: &str = "appearance.system_font";
    pub(super) const GLASS: &str = "appearance.glass";
    /// Message spacing, Group and Stack: `[density, grouped, stacked]`.
    pub(super) const MESSAGES: &str = "appearance.messages";
}

/// A write to the settings file that did not land; the setting keeps its
/// earlier value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotSaved;

/// The Appearance settings and where they are saved.
pub(crate) struct AppearanceStore {
    appearance: Appearance,
    prefs: Prefs,
}

impl Default for AppearanceStore {
    /// The defaults, kept nowhere: before the settings file is read, and in
    /// tests.
    fn default() -> AppearanceStore {
        AppearanceStore::read(Prefs::memory())
    }
}

impl AppearanceStore {
    /// The settings in this user's settings file; a missing or unreadable
    /// value takes its default.
    pub(crate) fn load() -> AppearanceStore {
        AppearanceStore::read(Prefs::open(APP_ID))
    }

    /// The settings in `prefs`, for tests that need a file of their own
    /// (one that cannot be written, say).
    #[cfg(test)]
    pub(crate) fn from_prefs(prefs: Prefs) -> AppearanceStore {
        AppearanceStore::read(prefs)
    }

    fn read(prefs: Prefs) -> AppearanceStore {
        let defaults = Appearance::default();
        let background =
            prefs
                .get::<Vec<(f32, f32)>>(key::BACKGROUND)
                .map_or(defaults.background, |colours| {
                    let colours: Vec<BackgroundColour> = colours
                        .into_iter()
                        .map(|(hue_degrees, vividness)| BackgroundColour {
                            hue_degrees,
                            vividness,
                        })
                        .collect();
                    let strength = prefs
                        .get(key::STRENGTH)
                        .unwrap_or(defaults.background.strength);
                    Background::new(&colours, strength)
                });
        let shape = prefs
            .get::<[f32; 5]>(key::SHAPE)
            .map_or(defaults.shape, |values| {
                ShapeValue::ALL
                    .into_iter()
                    .zip(values)
                    .fold(defaults.shape, |shape, (value, to)| shape.with(value, to))
            });
        let glass = prefs
            .get::<f32>(key::GLASS)
            .map_or(defaults.glass, |glass| glass.clamp(0.0, 1.0));
        let appearance = Appearance {
            theme: prefs
                .get::<String>(key::THEME)
                .and_then(|name| theme_named(&name)),
            background,
            grain: prefs
                .get::<f32>(key::GRAIN)
                .map_or(defaults.grain, |grain| grain.clamp(0.0, 1.0)),
            accent: prefs
                .get::<String>(key::ACCENT)
                .and_then(|name| accent_named(&name))
                .unwrap_or(defaults.accent),
            messages: prefs
                .get::<(String, bool, bool)>(key::MESSAGES)
                .and_then(|(density, grouped, stacked)| {
                    Some(MessageLayout {
                        density: density_named(&density)?,
                        grouped,
                        stacked,
                    })
                })
                .unwrap_or(defaults.messages),
            shape,
            system_font: prefs.get(key::SYSTEM_FONT).unwrap_or(defaults.system_font),
            glass,
        };
        AppearanceStore { appearance, prefs }
    }

    pub(crate) fn appearance(&self) -> &Appearance {
        &self.appearance
    }

    /// Saves `appearance` and keeps it; a failed write keeps the earlier
    /// settings. Writes only what changed.
    pub(crate) fn set(&mut self, appearance: Appearance) -> Result<(), NotSaved> {
        let old = self.appearance;
        crate::device_settings::reload(&mut self.prefs);
        let mut saved = true;
        // Each key is its own write; after a failed one nothing more is
        // tried, and the change is not kept.
        macro_rules! write {
            ($key:expr, $value:expr) => {
                if saved {
                    saved = self.prefs.set($key, $value).is_ok();
                }
            };
        }
        if let Some(theme) = appearance.theme.filter(|_| appearance.theme != old.theme) {
            write!(key::THEME, theme_name(theme));
        }
        if appearance.background != old.background {
            let colours: Vec<(f32, f32)> = appearance
                .background
                .colours()
                .iter()
                .map(|colour| (colour.hue_degrees, colour.vividness))
                .collect();
            write!(key::BACKGROUND, &colours);
            write!(key::STRENGTH, &appearance.background.strength);
        }
        if appearance.grain.to_bits() != old.grain.to_bits() {
            write!(key::GRAIN, &appearance.grain);
        }
        if appearance.accent != old.accent {
            write!(key::ACCENT, accent_name(appearance.accent));
        }
        if appearance.messages != old.messages {
            let messages = appearance.messages;
            let value = (
                density_name(messages.density),
                messages.grouped,
                messages.stacked,
            );
            write!(key::MESSAGES, &value);
        }
        if appearance.shape != old.shape {
            let values = ShapeValue::ALL.map(|value| appearance.shape.get(value));
            write!(key::SHAPE, &values);
        }
        if appearance.system_font != old.system_font {
            write!(key::SYSTEM_FONT, &appearance.system_font);
        }
        if appearance.glass.to_bits() != old.glass.to_bits() {
            write!(key::GLASS, &appearance.glass);
        }
        if !saved {
            log::warn!("an Appearance setting was not saved");
            return Err(NotSaved);
        }
        self.appearance = appearance;
        Ok(())
    }
}

fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Oled => "oled",
        Theme::Graphite => "graphite",
        Theme::Light => "light",
        Theme::SoftLight => "soft_light",
    }
}

fn theme_named(name: &str) -> Option<Theme> {
    Theme::ALL
        .into_iter()
        .find(|theme| theme_name(*theme) == name)
}

fn accent_name(accent: Accent) -> &'static str {
    match accent {
        Accent::Blue => "blue",
        Accent::Indigo => "indigo",
        Accent::Teal => "teal",
        Accent::Green => "green",
        Accent::Pink => "pink",
        Accent::Orange => "orange",
    }
}

fn accent_named(name: &str) -> Option<Accent> {
    Accent::ALL
        .into_iter()
        .find(|accent| accent_name(*accent) == name)
}

fn density_name(density: Density) -> &'static str {
    match density {
        Density::Compact => "compact",
        Density::Normal => "normal",
        Density::Comfortable => "comfortable",
        Density::ExtraCompact => "extra_compact",
    }
}

fn density_named(name: &str) -> Option<Density> {
    Density::ALL
        .into_iter()
        .find(|density| density_name(*density) == name)
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
