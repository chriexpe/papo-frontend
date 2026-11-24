//! A group of rows and each row's control (`docs/design.md` §8, Rows): the
//! tile (rows in a group are one block, 2 pt apart, outer corners 16 and
//! inner 4), a switch, a choice with its menu, a sub-page's ›, the accent
//! swatches and the starting-point chips; sliders are `slider`'s. Every control is
//! reached by the keyboard and named for assistive technology; a row whose
//! save failed shows one `danger` line under it with Try again.
//!
//! Owns how a row looks and what its input asks for (a `Change`, a page, a
//! menu); the shell carries them out.

use rune::{Color, El, col, el, icon, row, text};

use super::rows::{Group, HIGHLIGHT_HUES, Kind, Row, direction_name};
use super::{Change, RowId, colour_field};
use crate::appearance::Appearance;
use crate::appearance::presets::Direction;
use crate::design::settings::{
    CHIP_HEIGHT, GROUP_TITLE_GAP, KNOB, ROW_INSET_PT, ROW_MIN_HEIGHT, ROW_PADDING, SWATCH_PT,
    SWATCH_RADIUS, SWATCH_RING_PT, SWITCH_KNOB_PT, SWITCH_PT, TILE_INNER_RADIUS, TILE_OUTER_RADIUS,
};
use crate::design::{
    Accent, FIELD_OUTLINE_PT, FOCUS_RING_PT, Look, ROW_ICON_PT, SPACE_LG, SPACE_MD, SPACE_SM,
    SPACE_XS, TYPE_BODY, TYPE_BODY_MEDIUM, TYPE_CAPTION, TYPE_SMALL, TYPE_SMALL_MEDIUM,
};
use crate::icons::{CHEVRON_DOWN, CHEVRON_RIGHT};
use crate::shell::Shell;
use crate::strings;

/// What a group needs besides its rows.
pub(super) struct Ctx<'a> {
    pub(super) look: &'a Look,
    /// The settings drawn (a drag's draft while one runs).
    pub(super) appearance: &'a Appearance,
    pub(super) failed: Option<RowId>,
    pub(super) colour: super::ColourState,
    /// The grain's tile for this theme, for Grain's slider.
    pub(super) grain: Option<&'a rune::Pixels>,
    /// Trusted domains' field, and whether what it held was refused.
    pub(super) domain: &'a rune::TextEdit,
    pub(super) domain_refused: bool,
}

/// A group: its optional title, then its rows as one block of tiles.
pub(super) fn group(group: Group, cx: &Ctx<'_>) -> El {
    let look = cx.look;
    let count = group.rows.len();
    let tiles = col().w_full().gap(look.space(SPACE_XS)).children(
        group
            .rows
            .into_iter()
            .enumerate()
            .map(|(index, row)| tile(row, index == 0, index + 1 == count, cx)),
    );
    let mut block = col().w_full().gap(look.space(GROUP_TITLE_GAP));
    if let Some(title) = group.title {
        block = block.child(
            el().px(look.space(SPACE_SM)).child(
                text(title)
                    .font(look.font(TYPE_SMALL_MEDIUM))
                    .color(look.palette.text_secondary)
                    .role("heading")
                    .level(3),
            ),
        );
    }
    block.child(tiles)
}

/// One row on its tile, and its danger line when its save failed.
pub(super) fn tile(setting: Row, first: bool, last: bool, cx: &Ctx<'_>) -> El {
    let look = cx.look;
    let (outer, inner) = (
        look.radius(TILE_OUTER_RADIUS),
        look.radius(TILE_INNER_RADIUS),
    );
    let top = if first { outer } else { inner };
    let bottom = if last { outer } else { inner };
    let failed = cx.failed == Some(setting.id);
    let refused = matches!(setting.kind, Kind::DomainField) && cx.domain_refused;
    let body = match setting.kind {
        Kind::Slider(slider) => super::slider::view(&setting, slider, cx),
        Kind::ColourField => colour_field::view(cx.appearance, cx.colour, look),
        Kind::Directions(current) => directions(&setting, current, look),
        Kind::DomainField => super::domain_field::view(cx.domain, look),
        Kind::Destructive { .. } => super::actions::destructive(setting, look),
        _ => plain_row(setting, look),
    };
    let corners = [top, top, bottom, bottom];
    // The row takes its tile's corners too, so its focus highlight and
    // ring follow them rather than squaring them off.
    let tile = el()
        .w_full()
        .corners(corners)
        .fill(look.palette.surface_tile)
        .child(body.corners(corners));
    if refused {
        return col().w_full().gap(look.space(SPACE_XS)).child(tile).child(
            el().px(ROW_INSET_PT).role("alert").child(
                text(strings::settings_trusted_invalid())
                    .font(look.font(TYPE_SMALL))
                    .color(look.palette.danger),
            ),
        );
    }
    if !failed {
        return tile;
    }
    col().w_full().gap(look.space(SPACE_XS)).child(tile).child(
        row()
            .px(ROW_INSET_PT)
            .gap(look.space(SPACE_MD))
            .role("alert")
            .child(
                text(strings::settings_save_failed())
                    .font(look.font(TYPE_SMALL))
                    .color(look.palette.danger),
            )
            .child(
                text(strings::settings_save_retry())
                    .key("settings-retry")
                    .font(look.font(TYPE_SMALL_MEDIUM))
                    .color(look.palette.danger)
                    .role("button")
                    .focusable(true)
                    .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                    .on_press_tx(|shell: &mut Shell, tx| shell.settings_retry(tx)),
            ),
    )
}

/// A row of title, summary and its control at the end: a switch, a
/// choice, a sub-page or the accent swatches.
fn plain_row(setting: Row, look: &Look) -> El {
    let Row {
        id,
        title,
        sub,
        kind,
    } = setting;
    let mut words = col().grow(1.0).min_w(0.0).child(
        text(title.clone())
            .font(look.font(TYPE_BODY_MEDIUM))
            .color(look.palette.text_primary),
    );
    if let Some(sub) = &sub {
        words = words.child(
            text(sub.clone())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted),
        );
    }
    let line = row()
        .key(id.key())
        .w_full()
        .min_h(look.space(ROW_MIN_HEIGHT))
        .py(look.space(ROW_PADDING))
        .px(ROW_INSET_PT)
        .gap(look.space(SPACE_LG))
        .center_y()
        .child(words);
    let spoken = match &sub {
        Some(sub) => format!("{title}, {sub}"),
        None => title.clone(),
    };
    match kind {
        Kind::Switch(on) => pressable(line, look)
            .role("switch")
            .checked(on)
            .label(spoken)
            .on_press_tx(move |shell: &mut Shell, tx| {
                shell.settings_change(Change::Switch(id, !on), tx);
            })
            .child(switch(on, look)),
        Kind::Choice(value) => pressable(line, look)
            .role("button")
            .label(format!("{spoken}, {value}"))
            .on_press_tx({
                let drawn = look.theme();
                move |shell: &mut Shell, tx| shell.settings_choice(id, drawn, tx)
            })
            .child(value_text(&value, look))
            .child(icon(&CHEVRON_DOWN, ROW_ICON_PT).color(look.palette.text_muted)),
        Kind::SubPage(page, value) => pressable(line, look)
            .role("link")
            .label(format!("{spoken}, {value}"))
            .on_press_tx(move |shell: &mut Shell, tx| shell.settings_page(page, tx))
            .child(value_text(&value, look))
            .child(icon(&CHEVRON_RIGHT, ROW_ICON_PT).color(look.palette.text_muted)),
        Kind::Accents(current) => line.label(spoken).child(accents(current, look)),
        Kind::Hues(current) => line
            .label(spoken.clone())
            .child(hues(id, &spoken, current, look)),
        Kind::Remove(host) => line
            .label(spoken)
            .child(super::actions::remove_button(id, &host, look)),
        Kind::Note | Kind::Info => line.role("status").label(spoken),
        Kind::Value(value) => line
            .label(format!("{spoken}, {value}"))
            .child(value_text(&value, look)),
        Kind::Action => pressable(line, look)
            .role("button")
            .label(spoken)
            .on_press_tx(move |shell: &mut Shell, tx| shell.settings_action(id, tx)),
        Kind::Slider(_)
        | Kind::ColourField
        | Kind::Directions(_)
        | Kind::DomainField
        | Kind::Destructive { .. } => line,
    }
}

/// A row that is one control: it takes the keyboard and the pointer, and
/// lights under them.
pub(super) fn pressable(line: El, look: &Look) -> El {
    line.focusable(true)
        .focus_highlight(look.palette.surface_hover)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
}

/// A choice's or sub-page's current value, before its chevron.
fn value_text(value: &str, look: &Look) -> El {
    text(value.to_owned())
        .font(look.font(TYPE_BODY))
        .color(look.palette.text_secondary)
        .max_lines(1)
}

/// A switch: the accent when on, a strong line when off, the knob at its
/// end.
fn switch(on: bool, look: &Look) -> El {
    let (width, height) = SWITCH_PT;
    let inset = (height - SWITCH_KNOB_PT) / 2.0;
    let track = if on {
        look.palette.accent
    } else {
        look.palette.line_strong
    };
    el().size(width, height)
        .radius(height / 2.0)
        .fill(track)
        .a11y_hidden()
        .child(
            el().absolute(
                if on {
                    width - inset - SWITCH_KNOB_PT
                } else {
                    inset
                },
                inset,
            )
            .size(SWITCH_KNOB_PT, SWITCH_KNOB_PT)
            .radius(SWITCH_KNOB_PT / 2.0)
            .fill(KNOB),
        )
}

/// The accent swatches: one per accent, the chosen one ringed.
fn accents(current: Accent, look: &Look) -> El {
    let dark = look.theme().is_dark();
    row()
        .gap(look.space(SPACE_MD))
        .role("radiogroup")
        .label(strings::settings_accent())
        .focus_group()
        .children(Accent::ALL.into_iter().map(|accent| {
            let lightness = if dark { 0.70 } else { 0.555 };
            let colour = rune::oklch(lightness, 0.18, accent.hue_degrees());
            let chosen = accent == current;
            el().key(format!("settings-accent-{accent:?}"))
                .size(SWATCH_PT, SWATCH_PT)
                .radius(look.radius(SWATCH_RADIUS))
                .fill(colour)
                .border(
                    SWATCH_RING_PT,
                    if chosen {
                        look.palette.text_primary
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .role("radio")
                .checked(chosen)
                .label(super::rows::accent_name(accent))
                .focusable(true)
                .focus_selected(chosen)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press_tx(move |shell: &mut Shell, tx| {
                    shell.settings_change(Change::Accent(accent), tx);
                })
        }))
}

/// A highlight's hue swatches (Message highlights): one per hue in
/// `HIGHLIGHT_HUES`, the chosen one ringed.
fn hues(id: RowId, name: &str, current: f32, look: &Look) -> El {
    let dark = look.theme().is_dark();
    row()
        .gap(look.space(SPACE_MD))
        .role("radiogroup")
        .label(name.to_owned())
        .focus_group()
        .children(HIGHLIGHT_HUES.into_iter().map(|hue| {
            let lightness = if dark { 0.70 } else { 0.555 };
            let colour = rune::oklch(lightness, 0.15, hue);
            let chosen = (hue - current).abs() < 0.5;
            let degrees = strings::decimal(hue, 0);
            el().key(format!("{}-{degrees}", id.key()))
                .size(SWATCH_PT, SWATCH_PT)
                .radius(look.radius(SWATCH_RADIUS))
                .fill(colour)
                .border(
                    SWATCH_RING_PT,
                    if chosen {
                        look.palette.text_primary
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .role("radio")
                .checked(chosen)
                .label(strings::settings_hue_spoken(&degrees))
                .focusable(true)
                .focus_selected(chosen)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press_tx(move |shell: &mut Shell, tx| {
                    shell.settings_change(Change::Hue(id, hue), tx);
                })
        }))
}

/// Shape and type's starting points as three chips, the one the values
/// match pressed.
fn directions(setting: &Row, current: Option<Direction>, look: &Look) -> El {
    el().w_full().padding(look.space(SPACE_MD)).child(
        row()
            .key(setting.id.key())
            .w_full()
            .gap(look.space(SPACE_SM))
            .role("radiogroup")
            .label(setting.title.clone())
            .focus_group()
            .children(Direction::ALL.into_iter().map(|direction| {
                let chosen = current == Some(direction);
                let (fill, ink) = if chosen {
                    (look.palette.accent_soft, look.palette.link)
                } else {
                    (look.palette.surface_tile, look.palette.text_secondary)
                };
                el().key(format!("settings-direction-{direction:?}"))
                    .grow(1.0)
                    .h(look.space(CHIP_HEIGHT))
                    .center()
                    // Concentric with the tile, alone in its group so
                    // round at every corner, one `space.md` out.
                    .radius(Look::nested_radius(
                        look.radius(TILE_OUTER_RADIUS),
                        look.space(SPACE_MD),
                    ))
                    .fill(fill)
                    .border(FIELD_OUTLINE_PT, look.palette.line_subtle)
                    .role("radio")
                    .checked(chosen)
                    .label(direction_name(direction))
                    .focusable(true)
                    .focus_selected(chosen)
                    .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                    .on_press_tx(move |shell: &mut Shell, tx| {
                        shell.settings_change(Change::Direction(direction), tx);
                    })
                    .child(
                        text(direction_name(direction))
                            .font(look.font(TYPE_SMALL_MEDIUM))
                            .color(ink),
                    )
            })),
    )
}
