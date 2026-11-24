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
use super::chip::{Tone, chip};
use super::controls::pressable;
use super::rows::{Kind, Row};
use crate::design::settings::{ROW_INSET_PT, ROW_MIN_HEIGHT, ROW_PADDING};
use crate::design::{Look, SPACE_LG, TYPE_BODY_MEDIUM, TYPE_CAPTION};
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
            .child(chip(
                format!("{}-cancel", id.key()),
                strings::button_cancel(),
                strings::button_cancel(),
                Tone::Plain,
                move |shell: &mut Shell, _tx| shell.settings_cancel_ask(),
                look,
            ))
            .child(chip(
                format!("{}-confirm", id.key()),
                &confirm,
                &confirm,
                Tone::Danger,
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

/// A trusted domain's Remove.
pub(super) fn remove_button(id: RowId, host: &str, look: &Look) -> El {
    let host = host.to_owned();
    chip(
        format!("{}-remove", id.key()),
        strings::settings_trusted_remove(),
        &strings::settings_trusted_remove_spoken(&host),
        Tone::Plain,
        move |shell: &mut Shell, tx| shell.settings_untrust(&host, tx),
        look,
    )
}

/// A blocked person's Unblock, at once (§8: undone the same way).
pub(super) fn unblock_button(id: RowId, user: papo_sdk::Uuid, name: &str, look: &Look) -> El {
    chip(
        format!("{}-unblock", id.key()),
        strings::card_unblock(),
        &strings::direct_unblock(name),
        Tone::Plain,
        move |shell: &mut Shell, tx| shell.settings_unblock(user, tx),
        look,
    )
}
