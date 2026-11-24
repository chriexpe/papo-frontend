//! The Appearance page and its two sub-pages (`docs/design.md` §4 to §6,
//! §8): Theme, Background colour, Accent; Message spacing, Group and Stack;
//! Shape and type, the system font and Glass; the colour field with its
//! strength and grain; Shape and type's starting points and sliders. With
//! the names their values show.
//!
//! Owns these pages' rows; the settings are `appearance`'s.

use super::{Context, Group, Kind, Row, Slider, Track};
use crate::appearance::presets::{Direction, Preset};
use crate::appearance::{Appearance, ShapeValue};
use crate::design::{Accent, Density, MessageLayout, Theme};
use crate::settings::{PageId, RowId};
use crate::strings;

pub(super) fn appearance(cx: &Context<'_>) -> Vec<Group> {
    let appearance = cx.appearance;
    let theme = appearance.theme_or(cx.system_dark);
    let messages = appearance.messages;
    let mut message_rows = vec![
        Row {
            id: RowId::Density,
            title: strings::settings_density().to_owned(),
            sub: None,
            kind: Kind::Choice(density_name(messages.density).to_owned()),
        },
        Row {
            id: RowId::Grouped,
            title: strings::settings_grouped().to_owned(),
            sub: None,
            kind: Kind::Switch(messages.grouped),
        },
    ];
    // Stack only means something without avatars (§6, §8): at Extra
    // compact, or with Show avatars off. An item that does nothing is left
    // out, not greyed.
    let layout = MessageLayout {
        avatars: cx.conversations.show_avatars,
        ..messages
    };
    if !layout.has_avatars() {
        message_rows.push(Row {
            id: RowId::Stacked,
            title: strings::settings_stacked().to_owned(),
            sub: Some(strings::settings_stacked_sub().to_owned()),
            kind: Kind::Switch(messages.stacked),
        });
    }
    let mut shape_rows = vec![Row {
        id: RowId::Shape,
        title: strings::settings_shape().to_owned(),
        sub: None,
        kind: Kind::SubPage(PageId::Shape, direction_summary(appearance)),
    }];
    if cx.system_font_known {
        shape_rows.push(Row {
            id: RowId::SystemFont,
            title: strings::settings_system_font().to_owned(),
            sub: Some(strings::settings_system_font_sub().to_owned()),
            kind: Kind::Switch(appearance.system_font),
        });
    }
    shape_rows.push(Row {
        id: RowId::Glass,
        title: strings::settings_glass().to_owned(),
        sub: Some(strings::settings_glass_sub().to_owned()),
        kind: Kind::Slider(Slider {
            value: appearance.glass,
            min: 0.0,
            max: 1.0,
            step: 0.05,
            track: Track::Plain,
        }),
    });
    vec![
        Group {
            title: None,
            rows: vec![
                Row {
                    id: RowId::Theme,
                    title: strings::settings_theme().to_owned(),
                    sub: None,
                    kind: Kind::Choice(theme_name(theme).to_owned()),
                },
                Row {
                    id: RowId::Background,
                    title: strings::settings_background().to_owned(),
                    sub: None,
                    kind: Kind::SubPage(PageId::Background, background_summary(appearance)),
                },
                Row {
                    id: RowId::Accent,
                    title: strings::settings_accent().to_owned(),
                    sub: None,
                    kind: Kind::Accents(appearance.accent),
                },
            ],
        },
        Group {
            title: Some(strings::settings_group_messages()),
            rows: message_rows,
        },
        Group {
            title: Some(strings::settings_group_shape()),
            rows: shape_rows,
        },
    ]
}

pub(super) fn background(cx: &Context<'_>) -> Vec<Group> {
    let appearance = cx.appearance;
    let strength = Row {
        id: RowId::Strength,
        title: strings::settings_strength().to_owned(),
        sub: None,
        kind: Kind::Slider(Slider {
            value: appearance.background.strength,
            min: 0.0,
            max: 1.0,
            step: 0.05,
            track: Track::Strength,
        }),
    };
    let oled = appearance.theme_or(cx.system_dark) == Theme::Oled;
    let grain = Row {
        id: RowId::Grain,
        title: strings::settings_grain().to_owned(),
        sub: oled.then(|| strings::settings_grain_oled().to_owned()),
        kind: Kind::Slider(Slider {
            value: appearance.grain,
            min: 0.0,
            max: 1.0,
            step: 0.05,
            track: Track::Grain,
        }),
    };
    vec![
        Group {
            title: None,
            rows: vec![Row {
                id: RowId::Field,
                title: strings::settings_background().to_owned(),
                sub: None,
                kind: Kind::ColourField,
            }],
        },
        Group {
            title: None,
            rows: vec![strength, grain],
        },
    ]
}

pub(super) fn shape(cx: &Context<'_>) -> Vec<Group> {
    let shape = cx.appearance.shape;
    let sliders = ShapeValue::ALL
        .into_iter()
        .map(|value| {
            let (min, max) = value.range();
            Row {
                id: RowId::Slider(value),
                title: shape_value_name(value).to_owned(),
                sub: None,
                kind: Kind::Slider(Slider {
                    value: shape.get(value),
                    min,
                    max,
                    step: shape_step(value),
                    track: Track::Plain,
                }),
            }
        })
        .collect();
    vec![
        Group {
            title: None,
            rows: vec![Row {
                id: RowId::Directions,
                title: strings::settings_shape().to_owned(),
                sub: None,
                kind: Kind::Directions(Direction::matching(&shape)),
            }],
        },
        Group {
            title: None,
            rows: sliders,
        },
    ]
}

/// What one arrow key moves a Shape and type value by.
fn shape_step(value: ShapeValue) -> f32 {
    match value {
        ShapeValue::TextSize | ShapeValue::Blur => 1.0,
        ShapeValue::Roundness | ShapeValue::Spacing | ShapeValue::ShadowDepth => 0.05,
    }
}

/// A slider's value as the row shows it (§8: "14.0 px", "×1.00", "40%").
pub(crate) fn slider_value(id: RowId, value: f32) -> String {
    match id {
        RowId::Slider(ShapeValue::TextSize) => {
            strings::settings_value_px(&strings::decimal(value, 1))
        }
        RowId::Slider(ShapeValue::Blur) => strings::settings_value_px(&strings::decimal(value, 0)),
        RowId::Slider(_) => strings::settings_value_times(&strings::decimal(value, 2)),
        _ => strings::settings_value_percent(&strings::decimal(value * 100.0, 0)),
    }
}

/// The Appearance nav row's one line: "Graphite · Blue".
pub(crate) fn appearance_summary(appearance: &Appearance, system_dark: bool) -> String {
    strings::settings_appearance_summary(
        theme_name(appearance.theme_or(system_dark)),
        accent_name(appearance.accent),
    )
}

/// The Background colour row's value: the preset's name, else each
/// colour's hue, else "none".
fn background_summary(appearance: &Appearance) -> String {
    if let Some(preset) = Preset::matching(&appearance.background) {
        return crate::settings::colour_tools::preset_name(preset.id).to_owned();
    }
    let colours = appearance.background.colours();
    if colours.is_empty() {
        return strings::settings_background_none().to_owned();
    }
    colours
        .iter()
        .map(|colour| format!("{}°", colour.hue_degrees.round()))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// The Shape and type row's value: its starting point's name, or Custom.
fn direction_summary(appearance: &Appearance) -> String {
    Direction::matching(&appearance.shape)
        .map_or_else(strings::settings_shape_custom, direction_name)
        .to_owned()
}

pub(crate) fn theme_name(theme: Theme) -> &'static str {
    match theme {
        Theme::Oled => strings::theme_oled(),
        Theme::Graphite => strings::theme_graphite(),
        Theme::Light => strings::theme_light(),
        Theme::SoftLight => strings::theme_soft(),
    }
}

pub(crate) fn accent_name(accent: Accent) -> &'static str {
    match accent {
        Accent::Blue => strings::accent_blue(),
        Accent::Indigo => strings::accent_indigo(),
        Accent::Teal => strings::accent_teal(),
        Accent::Green => strings::accent_green(),
        Accent::Pink => strings::accent_pink(),
        Accent::Orange => strings::accent_orange(),
    }
}

pub(crate) fn density_name(density: Density) -> &'static str {
    match density {
        Density::Compact => strings::density_compact(),
        Density::Normal => strings::density_normal(),
        Density::Comfortable => strings::density_comfortable(),
        Density::ExtraCompact => strings::density_extra(),
    }
}

pub(crate) fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::Compact => strings::direction_compact(),
        Direction::Balanced => strings::direction_balanced(),
        Direction::Airy => strings::direction_airy(),
    }
}

fn shape_value_name(value: ShapeValue) -> &'static str {
    match value {
        ShapeValue::TextSize => strings::settings_text_size(),
        ShapeValue::Roundness => strings::settings_roundness(),
        ShapeValue::Spacing => strings::settings_spacing(),
        ShapeValue::Blur => strings::settings_blur(),
        ShapeValue::ShadowDepth => strings::settings_shadow(),
    }
}
