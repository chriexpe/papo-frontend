//! A text row (`docs/design.md` §8, Rows): its value and ✎; pressed, it
//! turns into a field in place with a character count, Cancel and Save,
//! Enter saving and Escape cancelling. While its save is in flight it
//! waits; it can be edited only once its value is confirmed (`editable`).
//!
//! Owns the row's look and input; saving is the shell's
//! (`Shell::settings_text_save`).

use rune::{El, Key, KeyEvent, TextEdit, col, icon, row, text, text_input};

use super::RowId;
use super::chip::{Tone, chip};
use super::controls::pressable;
use crate::design::settings::{ROW_INSET_PT, ROW_MIN_HEIGHT, ROW_PADDING};
use crate::design::{
    Look, ROW_ICON_PT, SPACE_LG, SPACE_MD, SPACE_SM, TYPE_BODY, TYPE_BODY_MEDIUM, TYPE_CAPTION,
};
use crate::icons::EDIT;
use crate::shell::Shell;
use crate::strings;

/// The field's element key.
pub(crate) fn field_key(id: RowId) -> String {
    format!("{}-field", id.key())
}

/// What a text row is drawn from.
pub(super) struct TextRow<'a> {
    pub(super) id: RowId,
    pub(super) title: String,
    pub(super) sub: Option<String>,
    pub(super) value: Option<String>,
    pub(super) placeholder: String,
    pub(super) most: usize,
    pub(super) editable: bool,
    /// The field, while the row is being edited.
    pub(super) editing: Option<&'a TextEdit>,
    /// Its save is in flight.
    pub(super) waiting: bool,
}

pub(super) fn view(row_of: TextRow<'_>, look: &Look) -> El {
    match row_of.editing {
        Some(edit) => editing(&row_of, edit, look),
        None => resting(row_of, look),
    }
}

fn resting(row_of: TextRow<'_>, look: &Look) -> El {
    let TextRow {
        id,
        title,
        sub,
        value,
        placeholder,
        editable,
        waiting,
        ..
    } = row_of;
    let shown = value.clone().unwrap_or(placeholder);
    let ink = if value.is_some() {
        look.palette.text_secondary
    } else {
        look.palette.text_muted
    };
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
    words = words.child(text(shown.clone()).font(look.font(TYPE_BODY)).color(ink));
    let line = row()
        .key(id.key())
        .w_full()
        .min_h(look.space(ROW_MIN_HEIGHT))
        .py(look.space(ROW_PADDING))
        .px(ROW_INSET_PT)
        .gap(look.space(SPACE_LG))
        .center_y()
        .child(words)
        .role("button")
        .label(format!("{title}, {shown}"))
        .busy(waiting);
    if !editable || waiting {
        return line;
    }
    pressable(line, look)
        .on_press_tx(move |shell: &mut Shell, tx| shell.settings_edit(id, tx))
        .child(icon(&EDIT, ROW_ICON_PT).color(look.palette.text_muted))
}

fn editing(row_of: &TextRow<'_>, edit: &TextEdit, look: &Look) -> El {
    let id = row_of.id;
    let count = edit.text.chars().count();
    let too_long = count > row_of.most;
    let field = text_input(edit, look.font(TYPE_BODY))
        .key(field_key(id))
        .single_line()
        .grow(1.0)
        .min_w(0.0)
        .color(look.palette.text_primary)
        .placeholder(row_of.placeholder.clone(), look.palette.text_muted)
        .caret(look.palette.accent, look.palette.text_selection)
        .label(row_of.title.clone())
        .invalid(too_long)
        .autofocus()
        .on_edit(Shell::settings_text_mut)
        .on_input_key(|shell: &mut Shell, key: &KeyEvent, tx| match key.key {
            Key::Enter => {
                shell.settings_text_save(tx);
                true
            }
            Key::Escape => {
                shell.settings_text_cancel(tx);
                true
            }
            _ => false,
        });
    let count_ink = if too_long {
        look.palette.danger
    } else {
        look.palette.text_muted
    };
    col()
        .key(id.key())
        .w_full()
        .py(look.space(ROW_PADDING))
        .px(ROW_INSET_PT)
        .gap(look.space(SPACE_SM))
        .child(
            text(row_of.title.clone())
                .font(look.font(TYPE_BODY_MEDIUM))
                .color(look.palette.text_primary),
        )
        .child(field)
        .child(
            row()
                .w_full()
                .gap(look.space(SPACE_MD))
                .center_y()
                .child(
                    text(strings::settings_char_count(count, row_of.most))
                        .grow(1.0)
                        .font(look.font(TYPE_CAPTION))
                        .color(count_ink),
                )
                .child(chip(
                    format!("{}-cancel", id.key()),
                    strings::button_cancel(),
                    strings::button_cancel(),
                    Tone::Plain,
                    |shell: &mut Shell, tx| shell.settings_text_cancel(tx),
                    look,
                ))
                .child(chip(
                    format!("{}-save", id.key()),
                    strings::settings_save(),
                    strings::settings_save(),
                    if too_long { Tone::Plain } else { Tone::Accent },
                    |shell: &mut Shell, tx| shell.settings_text_save(tx),
                    look,
                )),
        )
}
