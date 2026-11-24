//! Trusted domains' field (`docs/design.md` §8): type a domain, then Enter
//! or Add trusts it. What was typed is checked first
//! (`trusted_domains::typed_domain`); text that names no site stays in the
//! field with one `danger` line under it (`controls::tile`).
//!
//! Owns the field's look and input; adding is the shell's
//! (`Shell::settings_trust_typed`).

use rune::{El, Key, KeyEvent, TextEdit, row, text_input};

use super::chip::{Tone, chip};
use crate::design::settings::{ROW_INSET_PT, ROW_MIN_HEIGHT, ROW_PADDING};
use crate::design::{Look, SPACE_LG, TYPE_BODY};
use crate::settings::RowId;
use crate::shell::Shell;
use crate::strings;

/// The field's element key.
pub(crate) const DOMAIN_FIELD_KEY: &str = "settings-trusted-field";

/// The field and its Add, on the row's tile.
pub(super) fn view(domain: &TextEdit, look: &Look) -> El {
    let add = chip(
        format!("{}-add", RowId::TrustedAdd.key()),
        strings::settings_trusted_add(),
        strings::settings_trusted_add(),
        Tone::Accent,
        |shell: &mut Shell, tx| shell.settings_trust_typed(tx),
        look,
    );
    row()
        .key(RowId::TrustedAdd.key())
        .w_full()
        .min_h(look.space(ROW_MIN_HEIGHT))
        .py(look.space(ROW_PADDING))
        .px(ROW_INSET_PT)
        .gap(look.space(SPACE_LG))
        .center_y()
        .child(
            text_input(domain, look.font(TYPE_BODY))
                .key(DOMAIN_FIELD_KEY)
                .grow(1.0)
                .min_w(0.0)
                .color(look.palette.text_primary)
                .placeholder(
                    strings::settings_trusted_placeholder(),
                    look.palette.text_muted,
                )
                .caret(look.palette.accent, look.palette.text_selection)
                .label(strings::settings_trusted())
                .on_edit(Shell::settings_domain_mut)
                .on_input_key(|shell: &mut Shell, key: &KeyEvent, tx| {
                    if key.key == Key::Enter {
                        shell.settings_trust_typed(tx);
                        return true;
                    }
                    false
                }),
        )
        .child(add)
}
