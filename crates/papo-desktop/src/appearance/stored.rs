//! Where the Appearance settings sit in this device's settings file: one
//! key, `appearance`, holding every value in one JSON object. `rune::prefs`
//! rewrites the whole file on each `set`, so one key makes a change one
//! atomic write: a failed one leaves the file as it was, never with some of
//! a change's values written and others not. A value that is missing, of
//! the wrong kind or not a finite number takes its default.
//!
//! Owns the file's format; the values, their ranges and when they are
//! saved are `appearance`'s.

use rune::prefs::Prefs;
use serde_json::{Map, Value, json};

use super::{Appearance, ShapeValue};
use crate::design::{Accent, Background, BackgroundColour, Density, MessageLayout, Theme};

/// The settings file's key.
pub(super) const KEY: &str = "appearance";

/// The settings in `prefs`; the defaults for what is not there or not
/// readable.
pub(super) fn read(prefs: &Prefs) -> Appearance {
    let defaults = Appearance::default();
    let Some(Value::Object(stored)) = prefs.get::<Value>(KEY) else {
        return defaults;
    };
    let fraction = |name: &str, default: f32| {
        number(stored.get(name)).map_or(default, |value| value.clamp(0.0, 1.0))
    };
    Appearance {
        theme: stored
            .get("theme")
            .and_then(Value::as_str)
            .and_then(theme_named),
        background: background(&stored).unwrap_or(defaults.background),
        grain: fraction("grain", defaults.grain),
        accent: stored
            .get("accent")
            .and_then(Value::as_str)
            .and_then(accent_named)
            .unwrap_or(defaults.accent),
        messages: messages(stored.get("messages")).unwrap_or(defaults.messages),
        shape: shape(stored.get("shape"), defaults.shape),
        system_font: stored
            .get("system_font")
            .and_then(Value::as_bool)
            .unwrap_or(defaults.system_font),
        glass: fraction("glass", defaults.glass),
    }
}

/// The settings file's entry for `appearance`: its one key, so a change is
/// one write.
pub(super) fn entry(appearance: &Appearance) -> (&'static str, Value) {
    (KEY, value_of(appearance))
}

fn value_of(appearance: &Appearance) -> Value {
    let colours: Vec<[f32; 2]> = appearance
        .background
        .colours()
        .iter()
        .map(|colour| [colour.hue_degrees, colour.vividness])
        .collect();
    let messages = appearance.messages;
    json!({
        "theme": appearance.theme.map(theme_name),
        "background": colours,
        "strength": appearance.background.strength,
        "grain": appearance.grain,
        "accent": accent_name(appearance.accent),
        "messages": [density_name(messages.density), messages.grouped, messages.stacked],
        "shape": ShapeValue::ALL.map(|value| appearance.shape.get(value)),
        "system_font": appearance.system_font,
        "glass": appearance.glass,
    })
}

/// A finite number, or none.
fn number(value: Option<&Value>) -> Option<f32> {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a setting saved from an f32"
    )]
    let number = value?.as_f64()? as f32;
    number.is_finite().then_some(number)
}

fn background(stored: &Map<String, Value>) -> Option<Background> {
    let colours: Vec<BackgroundColour> = stored
        .get("background")?
        .as_array()?
        .iter()
        .filter_map(|pair| {
            let pair = pair.as_array()?;
            Some(BackgroundColour {
                hue_degrees: number(pair.first())?,
                vividness: number(pair.get(1))?,
            })
        })
        .collect();
    let strength = number(stored.get("strength"))?;
    Some(Background::new(&colours, strength))
}

fn messages(value: Option<&Value>) -> Option<MessageLayout> {
    let [density, grouped, stacked] = value?.as_array()?.as_slice() else {
        return None;
    };
    Some(MessageLayout {
        density: density_named(density.as_str()?)?,
        grouped: grouped.as_bool()?,
        stacked: stacked.as_bool()?,
    })
}

/// The saved values over `defaults`, each held to its range.
fn shape(value: Option<&Value>, defaults: super::Shape) -> super::Shape {
    let Some(values) = value.and_then(Value::as_array) else {
        return defaults;
    };
    ShapeValue::ALL
        .into_iter()
        .zip(values)
        .fold(defaults, |shape, (setting, value)| {
            number(Some(value)).map_or(shape, |to| shape.with(setting, to))
        })
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
