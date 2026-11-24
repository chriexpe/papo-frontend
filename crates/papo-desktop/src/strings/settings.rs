//! The settings panel's strings (`docs/design.md` §8) and the Appearance
//! page's (§4 to §6): the panel, its rows, their values and spoken labels,
//! and the numbers the sliders show, formatted for the locale. The catalog
//! entries are `settings.*`; the values' names are `appearance_names`'.

use super::{I18N, Key, catalog};

/// The panel's name: "Settings".
pub(crate) fn settings_title() -> &'static str {
    text!("settings.title")
}

/// The search field's placeholder and label.
pub(crate) fn settings_search() -> &'static str {
    text!("settings.search")
}

/// The search field's ✕.
pub(crate) fn settings_search_clear() -> &'static str {
    text!("settings.search_clear")
}

/// A search that matches no row.
pub(crate) fn settings_no_results() -> &'static str {
    text!("settings.no_results")
}

/// A sub-page's ←.
pub(crate) fn settings_back() -> &'static str {
    text!("settings.back")
}

/// The panel's ✕.
pub(crate) fn settings_close() -> &'static str {
    text!("settings.close")
}

/// The nav's group of the app's pages.
pub(crate) fn settings_group_app() -> &'static str {
    text!("settings.group_app")
}

/// A page's scope: its settings are this device's.
pub(crate) fn settings_scope_device() -> &'static str {
    text!("settings.scope_device")
}

/// The danger line under a row whose write failed.
pub(crate) fn settings_save_failed() -> &'static str {
    text!("settings.save_failed")
}

/// That line's action.
pub(crate) fn settings_save_retry() -> &'static str {
    text!("settings.save_retry")
}

/// The Appearance page.
pub(crate) fn settings_appearance() -> &'static str {
    text!("settings.appearance")
}

/// Theme.
pub(crate) fn settings_theme() -> &'static str {
    text!("settings.theme")
}

/// Background colour, the row and its page.
pub(crate) fn settings_background() -> &'static str {
    text!("settings.background")
}

/// Background colour's value with no colour.
pub(crate) fn settings_background_none() -> &'static str {
    text!("settings.background_none")
}

/// Accent.
pub(crate) fn settings_accent() -> &'static str {
    text!("settings.accent")
}

/// The Messages group.
pub(crate) fn settings_group_messages() -> &'static str {
    text!("settings.group_messages")
}

/// Message spacing.
pub(crate) fn settings_density() -> &'static str {
    text!("settings.density")
}

/// Group consecutive messages.
pub(crate) fn settings_grouped() -> &'static str {
    text!("settings.grouped")
}

/// Stack messages.
pub(crate) fn settings_stacked() -> &'static str {
    text!("settings.stacked")
}

/// Stack's summary.
pub(crate) fn settings_stacked_sub() -> &'static str {
    text!("settings.stacked_sub")
}

/// The Shape and type group.
pub(crate) fn settings_group_shape() -> &'static str {
    text!("settings.group_shape")
}

/// Shape and type, the row and its page.
pub(crate) fn settings_shape() -> &'static str {
    text!("settings.shape")
}

/// Shape and type's value when no starting point matches.
pub(crate) fn settings_shape_custom() -> &'static str {
    text!("settings.shape_custom")
}

/// Use the system font.
pub(crate) fn settings_system_font() -> &'static str {
    text!("settings.system_font")
}

/// Its summary.
pub(crate) fn settings_system_font_sub() -> &'static str {
    text!("settings.system_font_sub")
}

/// The Glass switch.
pub(crate) fn settings_glass() -> &'static str {
    text!("settings.glass")
}

/// Its summary.
pub(crate) fn settings_glass_sub() -> &'static str {
    text!("settings.glass_sub")
}

/// The colour field with no colour.
pub(crate) fn settings_field_empty() -> &'static str {
    text!("settings.field_empty")
}

/// The field's +.
pub(crate) fn settings_colour_add() -> &'static str {
    text!("settings.colour_add")
}

/// The field's −.
pub(crate) fn settings_colour_remove() -> &'static str {
    text!("settings.colour_remove")
}

/// The field's harmony action.
pub(crate) fn settings_harmony() -> &'static str {
    text!("settings.harmony")
}

/// The presets' ‹.
pub(crate) fn settings_presets_prev() -> &'static str {
    text!("settings.presets_prev")
}

/// The presets' ›.
pub(crate) fn settings_presets_next() -> &'static str {
    text!("settings.presets_next")
}

/// Colour strength.
pub(crate) fn settings_strength() -> &'static str {
    text!("settings.strength")
}

/// What assistive technology hears for the colour field.
pub(crate) fn settings_field_spoken() -> &'static str {
    text!("settings.field_spoken")
}

/// Grain.
pub(crate) fn settings_grain() -> &'static str {
    text!("settings.grain")
}

/// Grain's summary on OLED.
pub(crate) fn settings_grain_oled() -> &'static str {
    text!("settings.grain_oled")
}

/// Text size.
pub(crate) fn settings_text_size() -> &'static str {
    text!("settings.text_size")
}

/// Roundness.
pub(crate) fn settings_roundness() -> &'static str {
    text!("settings.roundness")
}

/// Spacing.
pub(crate) fn settings_spacing() -> &'static str {
    text!("settings.spacing")
}

/// Glass blur.
pub(crate) fn settings_blur() -> &'static str {
    text!("settings.blur")
}

/// Shadow depth.
pub(crate) fn settings_shadow() -> &'static str {
    text!("settings.shadow")
}

/// A search result's grouping line: "Appearance › Background colour".
pub(crate) fn settings_result_path(page: &str, row: &str) -> String {
    template!("settings.result_path", "page" => page, "row" => row)
}

/// The Appearance nav row's summary: "Graphite · Blue".
pub(crate) fn settings_appearance_summary(theme: &str, accent: &str) -> String {
    template!("settings.appearance_summary", "theme" => theme, "accent" => accent)
}

/// A colour on the field, for assistive technology: its place, hue and
/// vividness.
pub(crate) fn settings_colour_spoken(n: usize, hue_degrees: f32, vividness: f32) -> String {
    template!(
        "settings.colour_spoken",
        "n" => n.to_string(),
        "hue" => decimal(hue_degrees, 0),
        "percent" => decimal(vividness * 100.0, 0)
    )
}

/// A slider's value in points: "14.0 px".
pub(crate) fn settings_value_px(value: &str) -> String {
    template!("settings.value_px", "value" => value)
}

/// A multiplier: "×1.00".
pub(crate) fn settings_value_times(value: &str) -> String {
    template!("settings.value_times", "value" => value)
}

/// A share: "40%".
pub(crate) fn settings_value_percent(value: &str) -> String {
    template!("settings.value_percent", "value" => value)
}

/// `value` with `places` decimals, its decimal mark the locale's: a point
/// in en-US, a comma in pt-BR.
pub(crate) fn decimal(value: f32, places: usize) -> String {
    let written = format!("{value:.places$}");
    // Rounding can leave a negative zero ("-0"), which no setting means.
    let written =
        if written.starts_with('-') && written.trim_start_matches(['-', '0', '.']).is_empty() {
            written.trim_start_matches('-').to_owned()
        } else {
            written
        };
    if I18N.locales()[I18N.locale()].id() == "pt-BR" {
        written.replace('.', ",")
    } else {
        written
    }
}
