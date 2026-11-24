//! Change password's form (`docs/design.md` §8: a small form with its own
//! Save): New password and Repeat password, Show or Hide, §7's hint, one
//! line for what Save found wrong, and the note that only this account on
//! this server changes. While the change is in flight the form waits.
//!
//! Owns the form's look and input; checking and sending are the shell's
//! (`Shell::settings_password_save`).

use rune::{El, Key, KeyEvent, TextEdit, col, row, text, text_input};

use super::chip::{Tone, chip};
use super::{PasswordForm, PasswordProblem, RowId};
use crate::design::settings::{ROW_INSET_PT, ROW_PADDING, TILE_OUTER_RADIUS};
use crate::design::{
    FIELD_OUTLINE_PT, Look, SPACE_MD, SPACE_SM, TYPE_BODY, TYPE_CAPTION, TYPE_SMALL,
    TYPE_SMALL_MEDIUM,
};
use crate::shell::Shell;
use crate::strings;

/// The fields' element keys.
pub(crate) const NEW_PASSWORD_KEY: &str = "settings-new-password";
pub(crate) const REPEAT_PASSWORD_KEY: &str = "settings-repeat-password";

/// The form, on its tile; `note` is the row's summary.
pub(super) fn view(form: &PasswordForm, note: Option<String>, waiting: bool, look: &Look) -> El {
    let (hint, hint_ink) = match form.problem {
        Some(PasswordProblem::Mismatch) => {
            (strings::settings_password_mismatch(), look.palette.danger)
        }
        Some(PasswordProblem::Weak) => (strings::password_hint(), look.palette.danger),
        None => (strings::password_hint(), look.palette.text_muted),
    };
    let mut body = col()
        .key(RowId::PasswordForm.key())
        .w_full()
        .py(look.space(ROW_PADDING))
        .px(ROW_INSET_PT)
        .gap(look.space(SPACE_MD))
        .busy(waiting)
        .child(field(form, Which::New, waiting, look))
        .child(field(form, Which::Repeat, waiting, look))
        .child(text(hint).font(look.font(TYPE_SMALL)).color(hint_ink).role(
            if form.problem.is_some() {
                "alert"
            } else {
                "note"
            },
        ));
    if let Some(note) = note {
        body = body.child(
            text(note)
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted),
        );
    }
    body.child(
        row()
            .w_full()
            .gap(look.space(SPACE_MD))
            .justify_end()
            .child(chip(
                "settings-password-reveal".to_owned(),
                if form.shown {
                    strings::hide()
                } else {
                    strings::show()
                },
                if form.shown {
                    strings::hide_password()
                } else {
                    strings::show_password()
                },
                Tone::Plain,
                |shell: &mut Shell, _tx| shell.settings_password_reveal(),
                look,
            ))
            .child(chip(
                "settings-password-save".to_owned(),
                if waiting {
                    strings::settings_saving()
                } else {
                    strings::settings_save()
                },
                strings::settings_save(),
                if waiting { Tone::Plain } else { Tone::Accent },
                |shell: &mut Shell, tx| shell.settings_password_save(tx),
                look,
            )),
    )
}

/// A field with its label over it.
fn field(form: &PasswordForm, which: Which, waiting: bool, look: &Look) -> El {
    let (key, label, edit, access): (_, _, _, fn(&mut Shell) -> &mut TextEdit) = match which {
        Which::New => (
            NEW_PASSWORD_KEY,
            strings::settings_new_password(),
            &form.new,
            Shell::settings_new_password_mut,
        ),
        Which::Repeat => (
            REPEAT_PASSWORD_KEY,
            strings::settings_repeat_password(),
            &form.repeat,
            Shell::settings_repeat_password_mut,
        ),
    };
    let input = text_input(edit, look.font(TYPE_BODY))
        .key(key)
        .single_line()
        .secure()
        .revealed(form.shown)
        .grow(1.0)
        .min_w(0.0)
        .color(look.palette.text_primary)
        .caret(look.palette.accent, look.palette.text_selection)
        .label(label)
        .invalid(form.problem.is_some())
        .on_edit(access)
        .on_input_key(|shell: &mut Shell, key: &KeyEvent, tx| {
            if key.key == Key::Enter {
                shell.settings_password_save(tx);
                return true;
            }
            false
        });
    let input = if waiting { input.inert() } else { input };
    col()
        .w_full()
        .gap(look.space(SPACE_SM))
        .child(
            text(label)
                .font(look.font(TYPE_SMALL_MEDIUM))
                .color(look.palette.text_secondary),
        )
        .child(
            row()
                .w_full()
                .px(look.space(SPACE_MD))
                .py(look.space(SPACE_SM))
                .radius(Look::nested_radius(
                    look.radius(TILE_OUTER_RADIUS),
                    look.space(SPACE_MD),
                ))
                .border(FIELD_OUTLINE_PT, look.palette.line_strong)
                .child(input),
        )
}

/// Which field.
#[derive(Clone, Copy)]
enum Which {
    New,
    Repeat,
}
