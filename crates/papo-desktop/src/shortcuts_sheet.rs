//! The keyboard shortcuts sheet (`docs/design.md` §12; the design
//! reference's Shortcuts): Ctrl+/, F1 or Ctrl+? opens it over the dimmed app,
//! and Escape or the same key closes it. It lists the keys that work today,
//! grouped as the reference groups them, each with Papo's key and the
//! system's, a dash where there is none. A key that comes with a later
//! slice (notifications, settings, emoji, attachments, text size) joins the
//! sheet with it, never before.
//!
//! Owns the sheet's rows and look. Linux and Windows keys only, as the app
//! ships there first; the macOS column comes with the macOS build. What the
//! keys do is each surface's own.

use rune::{El, ScrollSpec, col, el, row, text};

use crate::design::{
    PILL_OUTLINE_PT, POPUP_PADDING_PT, POPUP_SHADOW_PT, Palette, RADIUS_INDICATOR_PT,
    RADIUS_POPUP_PT, SPACE_LG_PT, SPACE_MD_PT, SPACE_SM_PT, SPACE_XL_PT, SPACE_XXL_PT, TYPE_BODY,
    TYPE_CAPTION, TYPE_CAPTION_MEDIUM, TYPE_DISPLAY, TYPE_SMALL,
};
mod rows;

use rows::{Combo, Group, Shortcut, groups};

use crate::shell::Shell;
use crate::strings;

/// The element key of the open sheet.
pub(crate) const SHORTCUTS_KEY: &str = "shortcuts";
/// The element key of its scrolling rows, which take the keyboard.
pub(crate) const SHORTCUTS_ROWS_KEY: &str = "shortcuts-rows";

/// The sheet's width, in points.
const SHEET_WIDTH_PT: f32 = 720.0;
/// Each key column's width, in points.
const KEYS_COLUMN_PT: f32 = 190.0;
/// The tallest the sheet's rows grow before they scroll, in points.
const SHEET_MAX_HEIGHT_PT: f32 = 560.0;

/// The sheet over the dimmed app. Escape closes it.
pub(crate) fn view(palette: &Palette) -> El {
    let (drop, blur) = POPUP_SHADOW_PT;
    let mut body = col().w_full().gap(SPACE_XL_PT);
    for group in groups() {
        body = body.child(group_view(&group, palette));
    }
    let frame = col()
        .key(SHORTCUTS_KEY)
        .w(SHEET_WIDTH_PT)
        .padding(POPUP_PADDING_PT)
        .gap(SPACE_LG_PT)
        .radius(RADIUS_POPUP_PT)
        .fill(palette.surface_content)
        .border(PILL_OUTLINE_PT, palette.line_subtle)
        .shadow(drop, blur, palette.popup_shadow)
        .role("dialog")
        .label(strings::shortcuts_title())
        .modal(false, Shell::close_shortcuts)
        .child(
            text(strings::shortcuts_title())
                .font(TYPE_DISPLAY)
                .color(palette.text_primary)
                .role("heading")
                .level(2),
        )
        .child(
            // The keyboard sits on the rows, so ↑ ↓ and the page keys
            // scroll them.
            col()
                .key(SHORTCUTS_ROWS_KEY)
                .w_full()
                .max_h(SHEET_MAX_HEIGHT_PT)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .focusable(true)
                .autofocus()
                .child(body),
        );
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .padding(SPACE_XXL_PT)
        .fill(palette.scrim)
        .child(frame)
}

/// A group: its title over the Papo and System columns, then its rows.
fn group_view(group: &Group, palette: &Palette) -> El {
    let caption = |words: &str| {
        text(words.to_owned())
            .font(TYPE_CAPTION_MEDIUM)
            .color(palette.text_muted)
    };
    let head = row()
        .w_full()
        .gap(SPACE_MD_PT)
        .pb(SPACE_SM_PT)
        .child(
            text(group.title)
                .font(TYPE_BODY)
                .color(palette.text_primary)
                .role("heading")
                .level(3)
                .grow(1.0)
                .min_w(0.0),
        )
        .child(
            el().w(KEYS_COLUMN_PT)
                .child(caption(strings::shortcuts_papo())),
        )
        .child(
            el().w(KEYS_COLUMN_PT)
                .child(caption(strings::shortcuts_system())),
        );
    let mut table = col().w_full().role("list").label(group.title).child(head);
    for shortcut in &group.rows {
        // A hairline between rows.
        table = table
            .child(
                el().w_full()
                    .h(PILL_OUTLINE_PT)
                    .fill(palette.line_subtle)
                    .a11y_hidden(),
            )
            .child(shortcut_row(shortcut, palette));
    }
    table
}

/// One row: the action and its note, Papo's keys, the system's keys.
fn shortcut_row(shortcut: &Shortcut, palette: &Palette) -> El {
    let spoken = format!(
        "{}: {}, {}",
        shortcut.action,
        spoken_keys(shortcut.papo),
        spoken_keys(shortcut.system)
    );
    row()
        .w_full()
        .gap(SPACE_MD_PT)
        .py(SPACE_SM_PT)
        .items_start()
        .role("listitem")
        .label(spoken)
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .a11y_hidden()
                .child(
                    text(shortcut.action)
                        .font(TYPE_SMALL)
                        .color(palette.text_primary)
                        .wrap(),
                )
                .children(shortcut.note.map(|note| {
                    text(note)
                        .font(TYPE_CAPTION)
                        .color(palette.text_muted)
                        .wrap()
                })),
        )
        .child(keys_cell(shortcut.papo, palette))
        .child(keys_cell(shortcut.system, palette))
}

/// A column's keys: each combination as key caps, alternatives apart; a
/// dash with none.
fn keys_cell(combos: &[Combo], palette: &Palette) -> El {
    let mut cell = row()
        .w(KEYS_COLUMN_PT)
        .gap(SPACE_SM_PT)
        .wrap()
        .a11y_hidden();
    if combos.is_empty() {
        return cell.child(text("—").font(TYPE_SMALL).color(palette.text_muted));
    }
    for combo in combos {
        let mut caps = row().gap(SPACE_SM_PT / 2.0);
        for cap in *combo {
            caps = caps.child(
                el().px(SPACE_SM_PT)
                    .radius(RADIUS_INDICATOR_PT)
                    .fill(palette.surface_code)
                    .border(PILL_OUTLINE_PT, palette.line_subtle)
                    .child(text(*cap).font(TYPE_SMALL).color(palette.text_primary)),
            );
        }
        cell = cell.child(caps);
    }
    cell
}

/// A column's keys for assistive technology: "Ctrl K or F1", or none.
fn spoken_keys(combos: &[Combo]) -> String {
    if combos.is_empty() {
        return strings::shortcuts_none().to_owned();
    }
    combos
        .iter()
        .map(|combo| combo.join(" "))
        .collect::<Vec<_>>()
        .join(strings::shortcuts_or())
}
