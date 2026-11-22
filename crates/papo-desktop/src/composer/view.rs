//! The text pill (`docs/design.md` §3, Composer; the mock's `.comp`): the
//! field, growing to [`COMPOSER_MAX_LINES`] lines and then scrolling inside,
//! and Send, with the line over it that says the text is too long or a send
//! failed. Emoji, stickers and GIF join the pill with their pickers (D11).
//!
//! Pure: it reads the composer and hands Enter, Shift+Enter, edits and Send
//! to the shell.

use rune::{El, KeyEvent, PillStyle, auto_grow, col, icon, row, text, text_input};

use super::Composer;
use super::picker::row_key;
use crate::design::{
    COMPOSER_HEIGHT_PT, COMPOSER_MAX_LINES, FOCUS_RING_PT, PILL_ICON_PT, PILL_OUTLINE_PT, Palette,
    RADIUS_COMPOSER_PT, RADIUS_SEND_PT, SEND_BUTTON_PT, SPACE_LG_PT, SPACE_SM_PT, TYPE_BODY,
    TYPE_SMALL,
};
use crate::icons::SEND;
use crate::shell::Shell;
use crate::strings;

/// The element key of the composer's text field.
pub(crate) const COMPOSER_KEY: &str = "composer";

/// The opacity of Send while there is nothing to send.
const SEND_IDLE_OPACITY: f32 = 0.4;

/// The pill for the shown channel, named `channel`, with the line over it
/// when there is one. `enter_sends`: the "Enter to send" setting.
pub(crate) fn pill(composer: &Composer, channel: &str, enter_sends: bool, palette: &Palette) -> El {
    let Some((_, draft)) = composer.shown() else {
        return col();
    };
    let loading = composer.is_loading();
    let style = composer.field_style(palette);
    let (chip, ink) = (palette.accent_soft, palette.link);
    let mut field = text_input(&draft.edit, TYPE_BODY)
        .key(COMPOSER_KEY)
        .label(strings::composer_label())
        // A long link or word breaks at the field's edge instead of running
        // past it.
        .wrap_anywhere()
        .grow(1.0)
        .color(palette.text_primary)
        .placeholder(strings::composer_placeholder(channel), palette.text_muted)
        .caret(palette.accent, palette.text_selection)
        .spans(style.spans)
        .strikes(style.strikes)
        // Every pill is a mention, drawn as a message draws one.
        .pill_style(move |_| PillStyle::default().bg(chip).fg(ink));
    if let Some(picker) = composer.picker() {
        field = field.active_descendant(row_key(picker.selected()));
    }
    if loading {
        // No `on_edit` while the draft loads: typing waits for it.
        field = field.busy(true);
    } else {
        field = field
            .on_edit(|shell: &mut Shell| shell.composer.field_mut())
            .on_input_change(Shell::composer_edited)
            .on_selection_change(|shell: &mut Shell, _, _| shell.composer_moved())
            // With "Enter to send" on, plain Enter submits and Shift+Enter
            // is the editor's own line break.
            .on_submit(Shell::composer_send)
            .on_input_key(move |shell: &mut Shell, key: &KeyEvent, tx| {
                shell.composer_key(key, enter_sends, tx)
            });
    }
    let line_pt = TYPE_BODY.line_px();
    // One line sits level with Send; more lines grow the pill upward.
    let field = col()
        .grow(1.0)
        .min_w(0.0)
        .py(((SEND_BUTTON_PT - line_pt) / 2.0).max(0.0))
        .child(auto_grow(field, 1, COMPOSER_MAX_LINES));
    let send_inset_pt = (COMPOSER_HEIGHT_PT - SEND_BUTTON_PT) / 2.0;
    let pill = row()
        .key("composer-pill")
        .w_full()
        .min_h(COMPOSER_HEIGHT_PT)
        .pl(SPACE_LG_PT)
        .pr(send_inset_pt)
        .py(send_inset_pt)
        .gap(SPACE_SM_PT)
        .items_end()
        .radius(RADIUS_COMPOSER_PT)
        .fill(palette.surface_pill)
        .border(PILL_OUTLINE_PT, palette.line_pill)
        .child(field)
        .child(send_button(composer.can_send(), palette));
    let note = if composer.chars_over() > 0 {
        let over = u64::try_from(composer.chars_over()).unwrap_or(u64::MAX);
        Some(strings::composer_too_long(over))
    } else if composer.send_failed_here() {
        Some(strings::composer_send_failed().to_owned())
    } else {
        None
    };
    col()
        .key("composer-area")
        .w_full()
        .gap(SPACE_SM_PT)
        .children(note.map(|note| {
            text(note)
                .key("composer-note")
                .font(TYPE_SMALL)
                .color(palette.danger)
                .px(SPACE_LG_PT)
                .role("status")
                .live_region(rune::LiveRegion::Polite)
                .wrap()
        }))
        .child(pill)
}

/// Send: the accent square with the arrow, dimmed while there is nothing
/// to send.
fn send_button(enabled: bool, palette: &Palette) -> El {
    let button = row()
        .key("composer-send")
        .size(SEND_BUTTON_PT, SEND_BUTTON_PT)
        .center()
        .radius(RADIUS_SEND_PT)
        .fill(palette.accent)
        .role("button")
        .label(strings::composer_send())
        .disabled(!enabled)
        .child(icon(&SEND, PILL_ICON_PT).color(palette.accent_on));
    if enabled {
        button
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
            .on_press_tx(Shell::composer_send)
    } else {
        button.opacity(SEND_IDLE_OPACITY)
    }
}
