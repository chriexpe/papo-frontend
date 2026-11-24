//! The rows that act rather than set (`docs/design.md` §8, Rows): a
//! destructive row ("Asks inside the row": its title in `danger`; pressed,
//! the row turns into its question with Cancel and the action in red;
//! confirmed, it waits while the action runs), and a list item's Remove.
//!
//! Owns those rows' buttons and what their presses ask for; the shell
//! carries them out (`Shell::settings_ask`, `settings_cancel_ask`,
//! `settings_action`, `settings_untrust`).

use rune::{El, col, el, row, text};

use super::RowId;
use super::controls::pressable;
use super::rows::{Kind, Row};
use crate::design::settings::{
    CHIP_HEIGHT, ROW_INSET_PT, ROW_MIN_HEIGHT, ROW_PADDING, TILE_OUTER_RADIUS,
};
use crate::design::{
    FOCUS_RING_PT, Look, SPACE_LG, SPACE_MD, TYPE_BODY_MEDIUM, TYPE_CAPTION, TYPE_SMALL_MEDIUM,
};
use crate::shell::Shell;
use crate::strings;

/// The row: its title and summary, or, while it asks, its question with
/// Cancel and the confirmation.
pub(super) fn destructive(setting: Row, look: &Look) -> El {
    let Row {
        id,
        title,
        sub,
        kind,
    } = setting;
    let Kind::Destructive {
        question,
        confirm,
        asking,
        busy,
    } = kind
    else {
        return el();
    };
    let line = row()
        .key(id.key())
        .w_full()
        .min_h(look.space(ROW_MIN_HEIGHT))
        .py(look.space(ROW_PADDING))
        .px(ROW_INSET_PT)
        .gap(look.space(SPACE_LG))
        .center_y();
    if asking || busy {
        return line
            .role("group")
            .label(question.clone())
            .busy(busy)
            .child(
                text(question)
                    .grow(1.0)
                    .min_w(0.0)
                    .font(look.font(TYPE_BODY_MEDIUM))
                    .color(look.palette.text_primary),
            )
            .child(button(
                &format!("{}-cancel", id.key()),
                strings::button_cancel(),
                false,
                move |shell: &mut Shell, _tx| shell.settings_cancel_ask(),
                look,
            ))
            .child(button(
                &format!("{}-confirm", id.key()),
                &confirm,
                true,
                move |shell: &mut Shell, tx| shell.settings_action(id, tx),
                look,
            ));
    }
    let mut words = col().grow(1.0).min_w(0.0).child(
        text(title.clone())
            .font(look.font(TYPE_BODY_MEDIUM))
            .color(look.palette.danger),
    );
    if let Some(sub) = &sub {
        words = words.child(
            text(sub.clone())
                .font(look.font(TYPE_CAPTION))
                .color(look.palette.text_muted),
        );
    }
    let spoken = match &sub {
        Some(sub) => format!("{title}, {sub}"),
        None => title,
    };
    pressable(line, look)
        .role("button")
        .label(spoken)
        .on_press_tx(move |shell: &mut Shell, tx| shell.settings_ask(id, tx))
        .child(words)
}

/// Cancel, or the confirmation in red.
fn button(
    key: &str,
    label: &str,
    danger: bool,
    press: impl Fn(&mut Shell, &mut rune::Tx) + 'static,
    look: &Look,
) -> El {
    let (fill, ink) = if danger {
        (look.palette.danger, look.palette.danger_on)
    } else {
        (look.palette.surface_hover, look.palette.text_primary)
    };
    el().key(key.to_owned())
        .h(look.space(CHIP_HEIGHT))
        .px(look.space(SPACE_MD))
        .center()
        // As Shape and type's chips: concentric with a tile one `space.md`
        // out.
        .radius(Look::nested_radius(
            look.radius(TILE_OUTER_RADIUS),
            look.space(SPACE_MD),
        ))
        .fill(fill)
        .role("button")
        .label(label.to_owned())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(press)
        .child(
            text(label.to_owned())
                .font(look.font(TYPE_SMALL_MEDIUM))
                .color(ink),
        )
}

/// A list item's Remove: a small button in the row's end.
pub(super) fn remove_button(id: RowId, host: &str, look: &Look) -> El {
    let host = host.to_owned();
    el().key(format!("{}-remove", id.key()))
        .h(look.space(CHIP_HEIGHT))
        .px(look.space(SPACE_MD))
        .center()
        // As Shape and type's chips: concentric with a tile one
        // `space.md` out.
        .radius(Look::nested_radius(
            look.radius(TILE_OUTER_RADIUS),
            look.space(SPACE_MD),
        ))
        .fill(look.palette.surface_hover)
        .role("button")
        .label(strings::settings_trusted_remove_spoken(&host))
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(move |shell: &mut Shell, tx| shell.settings_untrust(&host, tx))
        .child(
            text(strings::settings_trusted_remove())
                .font(look.font(TYPE_SMALL_MEDIUM))
                .color(look.palette.text_primary),
        )
}
