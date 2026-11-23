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
    Look, PILL_OUTLINE_PT, POPUP_PADDING_PT, POPUP_SHADOW_PT, RADIUS_INDICATOR, RADIUS_POPUP,
    SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XL, SPACE_XXL, TYPE_BODY, TYPE_CAPTION,
    TYPE_CAPTION_MEDIUM, TYPE_DISPLAY, TYPE_SMALL,
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
pub(crate) fn view(look: &Look) -> El {
    let (drop, blur) = POPUP_SHADOW_PT;
    let mut body = col().w_full().gap(look.space(SPACE_XL));
    for group in groups() {
        body = body.child(group_view(&group, look));
    }
    let frame = col()
        .key(SHORTCUTS_KEY)
        .w(SHEET_WIDTH_PT)
        .padding(POPUP_PADDING_PT)
        .gap(look.space(SPACE_LG))
        .radius(look.radius(RADIUS_POPUP))
        .fill(look.palette.surface_content)
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .shadow(drop, blur, look.palette.popup_shadow)
        .role("dialog")
        .label(strings::shortcuts_title())
        .modal(false, Shell::close_shortcuts)
        .child(
            text(strings::shortcuts_title())
                .font(look.font(TYPE_DISPLAY))
                .color(look.palette.text_primary)
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
        .padding(look.space(SPACE_XXL))
        .fill(look.palette.scrim)
        .child(frame)
}

/// A group: its title over the Papo and System columns, then its rows.
fn group_view(group: &Group, look: &Look) -> El {
    let caption = |words: &str| {
        text(words.to_owned())
            .font(look.font(TYPE_CAPTION_MEDIUM))
            .color(look.palette.text_muted)
    };
    let head = row()
        .w_full()
        .gap(look.space(SPACE_MD))
        .pb(look.space(SPACE_SM))
        .child(
            text(group.title)
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_primary)
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
                    .fill(look.palette.line_subtle)
                    .a11y_hidden(),
            )
            .child(shortcut_row(shortcut, look));
    }
    table
}

/// One row: the action and its note, Papo's keys, the system's keys.
fn shortcut_row(shortcut: &Shortcut, look: &Look) -> El {
    let spoken = format!(
        "{}: {}, {}",
        shortcut.action,
        spoken_keys(shortcut.papo),
        spoken_keys(shortcut.system)
    );
    row()
        .w_full()
        .gap(look.space(SPACE_MD))
        .py(look.space(SPACE_SM))
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
                        .font(look.font(TYPE_SMALL))
                        .color(look.palette.text_primary)
                        .wrap(),
                )
                .children(shortcut.note.map(|note| {
                    text(note)
                        .font(look.font(TYPE_CAPTION))
                        .color(look.palette.text_muted)
                        .wrap()
                })),
        )
        .child(keys_cell(shortcut.papo, look))
        .child(keys_cell(shortcut.system, look))
}

/// A column's keys: each combination as key caps, alternatives apart; a
/// dash with none.
fn keys_cell(combos: &[Combo], look: &Look) -> El {
    let mut cell = row()
        .w(KEYS_COLUMN_PT)
        .gap(look.space(SPACE_SM))
        .wrap()
        .a11y_hidden();
    if combos.is_empty() {
        return cell.child(
            text("—")
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted),
        );
    }
    for combo in combos {
        let mut caps = row().gap(look.space(SPACE_SM) / 2.0);
        for cap in *combo {
            caps = caps.child(
                el().px(look.space(SPACE_SM))
                    .radius(look.radius(RADIUS_INDICATOR))
                    .fill(look.palette.surface_code)
                    .border(PILL_OUTLINE_PT, look.palette.line_subtle)
                    .child(
                        text(*cap)
                            .font(look.font(TYPE_SMALL))
                            .color(look.palette.text_primary),
                    ),
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
