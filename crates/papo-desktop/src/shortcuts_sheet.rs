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
use crate::shell::Shell;
use crate::strings;

/// The element key of the open sheet.
pub(crate) const SHORTCUTS_KEY: &str = "shortcuts";

/// The sheet's width, in points.
const SHEET_WIDTH_PT: f32 = 720.0;
/// Each key column's width, in points.
const KEYS_COLUMN_PT: f32 = 190.0;
/// The tallest the sheet's rows grow before they scroll, in points.
const SHEET_MAX_HEIGHT_PT: f32 = 560.0;

/// Keys pressed together, cap by cap ("Ctrl", "K").
type Combo = &'static [&'static str];

/// One row: what it does, a note under it, then Papo's keys and the
/// system's, each a list of alternatives.
struct Shortcut {
    action: &'static str,
    note: Option<&'static str>,
    papo: &'static [Combo],
    system: &'static [Combo],
}

/// A group of rows under its title.
struct Group {
    title: &'static str,
    rows: Vec<Shortcut>,
}

const fn row_of(
    action: &'static str,
    papo: &'static [Combo],
    system: &'static [Combo],
) -> Shortcut {
    Shortcut {
        action,
        note: None,
        papo,
        system,
    }
}

const fn noted(
    action: &'static str,
    note: &'static str,
    papo: &'static [Combo],
    system: &'static [Combo],
) -> Shortcut {
    Shortcut {
        action,
        note: Some(note),
        papo,
        system,
    }
}

/// Every group, in the reference's order.
fn groups() -> Vec<Group> {
    vec![
        anywhere(),
        panels(),
        composer(),
        message(),
        menus(),
        window(),
    ]
}

fn anywhere() -> Group {
    Group {
        title: strings::shortcuts_anywhere(),
        rows: vec![
            row_of(strings::shortcuts_jump(), &[&["Ctrl", "K"]], &[]),
            row_of(
                strings::shortcuts_server_number(),
                &[&["Ctrl", "1 – 9"]],
                &[],
            ),
            noted(
                strings::shortcuts_server_step(),
                strings::shortcuts_server_step_note(),
                &[],
                &[
                    &["Ctrl", "Tab"],
                    &["Ctrl", "⇧", "Tab"],
                    &["Ctrl", "PgDn"],
                    &["Ctrl", "PgUp"],
                ],
            ),
            row_of(
                strings::shortcuts_channel_step(),
                &[&["Alt", "↑"], &["Alt", "↓"]],
                &[],
            ),
            row_of(
                strings::shortcuts_unread_step(),
                &[&["Alt", "⇧", "↑"], &["Alt", "⇧", "↓"]],
                &[],
            ),
            row_of(
                strings::shortcuts_history(),
                &[],
                &[&["Alt", "←"], &["Alt", "→"]],
            ),
            row_of(strings::direct_title(), &[&["Ctrl", "⇧", "D"]], &[]),
            row_of(
                strings::shortcuts_title(),
                &[&["Ctrl", "/"]],
                &[&["F1"], &["Ctrl", "?"]],
            ),
            row_of(strings::shortcuts_region(), &[], &[&["F6"], &["⇧", "F6"]]),
        ],
    }
}

fn panels() -> Group {
    Group {
        title: strings::shortcuts_panels(),
        rows: vec![
            row_of(strings::search_title(), &[], &[&["Ctrl", "F"]]),
            row_of(strings::pins_title(), &[&["Ctrl", "P"]], &[]),
            row_of(strings::shortcuts_dm_profile(), &[&["Ctrl", "U"]], &[]),
            row_of(strings::shortcuts_close_top(), &[], &[&["Esc"]]),
        ],
    }
}

fn composer() -> Group {
    Group {
        title: strings::shortcuts_composer(),
        rows: vec![
            noted(
                strings::shortcuts_send(),
                strings::shortcuts_send_note(),
                &[&["Enter"]],
                &[],
            ),
            row_of(strings::shortcuts_send_always(), &[&["Ctrl", "Enter"]], &[]),
            noted(
                strings::shortcuts_into_conversation(),
                strings::shortcuts_into_conversation_note(),
                &[&["Ctrl", "↑"]],
                &[&["⇧", "Tab"], &["F6"]],
            ),
            noted(
                strings::shortcuts_edit_newest(),
                strings::shortcuts_field_empty(),
                &[&["↑"]],
                &[],
            ),
            row_of(strings::shortcuts_cancel_mode(), &[&["Esc"]], &[]),
            row_of(
                strings::shortcuts_bold_italic(),
                &[],
                &[&["Ctrl", "B"], &["Ctrl", "I"]],
            ),
            row_of(
                strings::shortcuts_undo(),
                &[],
                &[&["Ctrl", "Z"], &["Ctrl", "⇧", "Z"]],
            ),
        ],
    }
}

fn message() -> Group {
    Group {
        title: strings::shortcuts_message(),
        rows: vec![
            row_of(
                strings::shortcuts_move(),
                &[],
                &[&["↑"], &["↓"], &["Home"], &["End"]],
            ),
            row_of(
                strings::shortcuts_open_menu(),
                &[&["Enter"]],
                &[&["⇧", "F10"], &["Menu"]],
            ),
            row_of(
                strings::shortcuts_reply_edit_pin(),
                &[&["R"], &["E"], &["P"]],
                &[],
            ),
            row_of(
                strings::shortcuts_react_top(),
                &[&["1"], &["2"], &["3"]],
                &[],
            ),
            row_of(strings::shortcuts_add_reaction(), &[&["+"]], &[]),
            row_of(
                strings::shortcuts_copy_select(),
                &[],
                &[&["Ctrl", "C"], &["Ctrl", "A"]],
            ),
            row_of(strings::shortcuts_delete(), &[], &[&["Delete"]]),
            row_of(strings::shortcuts_back_to_composer(), &[&["Esc"]], &[]),
        ],
    }
}

fn menus() -> Group {
    Group {
        title: strings::shortcuts_menus(),
        rows: vec![
            row_of(
                strings::shortcuts_list_move(),
                &[],
                &[&["↑"], &["↓"], &["Home"], &["End"]],
            ),
            row_of(strings::shortcuts_letter(), &[], &[&["a – z"]]),
            row_of(
                strings::shortcuts_run(),
                &[],
                &[&["Enter"], &["Space"], &["→"]],
            ),
            row_of(strings::shortcuts_close_back(), &[], &[&["Esc"]]),
        ],
    }
}

fn window() -> Group {
    Group {
        title: strings::shortcuts_window(),
        rows: vec![
            row_of(strings::shortcuts_close_window(), &[], &[&["Ctrl", "W"]]),
            row_of(strings::shortcuts_quit(), &[], &[&["Ctrl", "Q"]]),
            row_of(strings::shortcuts_full_screen(), &[], &[&["F11"]]),
        ],
    }
}

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
        .focusable(true)
        .autofocus()
        .child(
            text(strings::shortcuts_title())
                .font(TYPE_DISPLAY)
                .color(palette.text_primary)
                .role("heading")
                .level(2),
        )
        .child(
            col()
                .w_full()
                .max_h(SHEET_MAX_HEIGHT_PT)
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
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
