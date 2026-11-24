//! Trusted domains' field (`docs/design.md` §8): type a domain, then Enter
//! or Add trusts it. What was typed is checked first
//! (`trusted_domains::typed_domain`); text that names no site stays in the
//! field with one `danger` line under it (`controls::tile`).
//!
//! Owns the field's look and input; adding is the shell's
//! (`Shell::settings_trust_typed`).

use rune::{El, Key, KeyEvent, TextEdit, el, row, text, text_input};

use crate::design::settings::{
    CHIP_HEIGHT, ROW_INSET_PT, ROW_MIN_HEIGHT, ROW_PADDING, TILE_OUTER_RADIUS,
};
use crate::design::{FOCUS_RING_PT, Look, SPACE_LG, SPACE_MD, TYPE_BODY, TYPE_SMALL_MEDIUM};
use crate::settings::RowId;
use crate::shell::Shell;
use crate::strings;

/// The field's element key.
pub(crate) const DOMAIN_FIELD_KEY: &str = "settings-trusted-field";

/// The field and its Add, on the row's tile.
pub(super) fn view(domain: &TextEdit, look: &Look) -> El {
    let add = el()
        .key(format!("{}-add", RowId::TrustedAdd.key()))
        .h(look.space(CHIP_HEIGHT))
        .px(look.space(SPACE_MD))
        .center()
        // As Shape and type's chips: concentric with a tile one `space.md`
        // out.
        .radius(Look::nested_radius(
            look.radius(TILE_OUTER_RADIUS),
            look.space(SPACE_MD),
        ))
        .fill(look.palette.accent)
        .role("button")
        .label(strings::settings_trusted_add())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(|shell: &mut Shell, tx| shell.settings_trust_typed(tx))
        .child(
            text(strings::settings_trusted_add())
                .font(look.font(TYPE_SMALL_MEDIUM))
                .color(look.palette.accent_on),
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
