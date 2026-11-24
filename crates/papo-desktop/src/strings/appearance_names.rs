//! The names of the Appearance settings' values (`docs/design.md` §4 to
//! §6): the themes, the accents, the message spacings, Shape and type's
//! starting points and the background presets. The catalog entries are
//! `theme.*`, `accent.*`, `density.*`, `direction.*` and `preset.*`.

use super::{I18N, Key, catalog};

/// The Compact starting point.
pub(crate) fn direction_compact() -> &'static str {
    text!("direction.compact")
}

/// The Balanced starting point.
pub(crate) fn direction_balanced() -> &'static str {
    text!("direction.balanced")
}

/// The Airy starting point.
pub(crate) fn direction_airy() -> &'static str {
    text!("direction.airy")
}

/// A theme's name.
pub(crate) fn theme_oled() -> &'static str {
    text!("theme.oled")
}

/// A theme's name.
pub(crate) fn theme_graphite() -> &'static str {
    text!("theme.graphite")
}

/// A theme's name.
pub(crate) fn theme_light() -> &'static str {
    text!("theme.light")
}

/// A theme's name.
pub(crate) fn theme_soft() -> &'static str {
    text!("theme.soft")
}

/// An accent's name.
pub(crate) fn accent_blue() -> &'static str {
    text!("accent.blue")
}

/// An accent's name.
pub(crate) fn accent_indigo() -> &'static str {
    text!("accent.indigo")
}

/// An accent's name.
pub(crate) fn accent_teal() -> &'static str {
    text!("accent.teal")
}

/// An accent's name.
pub(crate) fn accent_green() -> &'static str {
    text!("accent.green")
}

/// An accent's name.
pub(crate) fn accent_pink() -> &'static str {
    text!("accent.pink")
}

/// An accent's name.
pub(crate) fn accent_orange() -> &'static str {
    text!("accent.orange")
}

/// A density's name.
pub(crate) fn density_compact() -> &'static str {
    text!("density.compact")
}

/// A density's name.
pub(crate) fn density_normal() -> &'static str {
    text!("density.normal")
}

/// A density's name.
pub(crate) fn density_comfortable() -> &'static str {
    text!("density.comfortable")
}

/// A density's name.
pub(crate) fn density_extra() -> &'static str {
    text!("density.extra")
}

/// A background colour preset's name.
pub(crate) fn preset_neutral() -> &'static str {
    text!("preset.neutral")
}

/// A background colour preset's name.
pub(crate) fn preset_ink() -> &'static str {
    text!("preset.ink")
}

/// A background colour preset's name.
pub(crate) fn preset_slate() -> &'static str {
    text!("preset.slate")
}

/// A background colour preset's name.
pub(crate) fn preset_moss() -> &'static str {
    text!("preset.moss")
}

/// A background colour preset's name.
pub(crate) fn preset_teal() -> &'static str {
    text!("preset.teal")
}

/// A background colour preset's name.
pub(crate) fn preset_plum() -> &'static str {
    text!("preset.plum")
}

/// A background colour preset's name.
pub(crate) fn preset_rose() -> &'static str {
    text!("preset.rose")
}

/// A background colour preset's name.
pub(crate) fn preset_ember() -> &'static str {
    text!("preset.ember")
}

/// A background colour preset's name.
pub(crate) fn preset_sand() -> &'static str {
    text!("preset.sand")
}

/// A background colour preset's name.
pub(crate) fn preset_dusk() -> &'static str {
    text!("preset.dusk")
}

/// A background colour preset's name.
pub(crate) fn preset_lagoon() -> &'static str {
    text!("preset.lagoon")
}

/// A background colour preset's name.
pub(crate) fn preset_aurora() -> &'static str {
    text!("preset.aurora")
}

/// A background colour preset's name.
pub(crate) fn preset_sunset() -> &'static str {
    text!("preset.sunset")
}

/// A background colour preset's name.
pub(crate) fn preset_forest() -> &'static str {
    text!("preset.forest")
}

/// A background colour preset's name.
pub(crate) fn preset_monoblue() -> &'static str {
    text!("preset.monoblue")
}

/// A background colour preset's name.
pub(crate) fn preset_monogreen() -> &'static str {
    text!("preset.monogreen")
}
