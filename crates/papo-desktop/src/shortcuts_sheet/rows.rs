//! The shortcuts sheet's rows: every key that works today, grouped as the
//! design reference groups them, Papo's keys and the system's apart. A key
//! comes onto the sheet with the slice that makes it work.

use crate::strings;

/// Keys pressed together, cap by cap ("Ctrl", "K").
pub(super) type Combo = &'static [&'static str];

/// One row: what it does, a note under it, then Papo's keys and the
/// system's, each a list of alternatives.
pub(super) struct Shortcut {
    pub(super) action: &'static str,
    pub(super) note: Option<&'static str>,
    pub(super) papo: &'static [Combo],
    pub(super) system: &'static [Combo],
}

/// A group of rows under its title.
pub(super) struct Group {
    pub(super) title: &'static str,
    pub(super) rows: Vec<Shortcut>,
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
pub(super) fn groups() -> Vec<Group> {
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
