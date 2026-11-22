//! The confirmation popup: a question over the dimmed app with Cancel, the
//! default, and the action. Removing a server and sending a message again
//! ask through it.
//!
//! Owns the popup's layout and keys; what is asked and what each button
//! does is the caller's.

use rune::{El, Tx, col, el, row, text};

use crate::design::{
    CONFIRM_WIDTH_PT, FIELD_HEIGHT_PT, FIELD_OUTLINE_PT, FOCUS_RING_PT, PILL_OUTLINE_PT,
    POPUP_PADDING_PT, POPUP_SHADOW_PT, Palette, RADIUS_FIELD_PT, RADIUS_POPUP_PT, SPACE_LG_PT,
    SPACE_MD_PT, SPACE_XXL_PT, TYPE_BODY, TYPE_BODY_STRONG, TYPE_DISPLAY,
};
use crate::shell::Shell;

/// What a confirmation asks, and its buttons' keys and words.
pub(crate) struct Question<'a> {
    /// The popup's element key.
    pub(crate) key: &'static str,
    pub(crate) title: &'a str,
    pub(crate) body: &'a str,
    pub(crate) cancel_key: &'static str,
    pub(crate) cancel: &'static str,
    pub(crate) confirm_key: &'static str,
    pub(crate) confirm: &'static str,
    /// The action destroys something: drawn in the danger colour, else the
    /// accent.
    pub(crate) danger: bool,
}

/// `question` over the dimmed app. Escape, Return and Cancel call
/// `cancel`; only the action's button calls `confirm`.
pub(crate) fn confirm(
    question: &Question<'_>,
    cancel: fn(&mut Shell),
    confirm: fn(&mut Shell, &mut Tx),
    palette: &Palette,
) -> El {
    let (drop, blur) = POPUP_SHADOW_PT;
    let cancel_button = button(question.cancel_key, question.cancel, palette)
        .border(FIELD_OUTLINE_PT, palette.line_strong)
        .on_press(cancel)
        // Return cancels: the action is never the default.
        .default_button()
        .child(
            text(question.cancel)
                .font(TYPE_BODY_STRONG)
                .color(palette.text_primary),
        );
    let (fill, ink) = if question.danger {
        (palette.danger, palette.danger_on)
    } else {
        (palette.accent, palette.accent_on)
    };
    let confirm_button = button(question.confirm_key, question.confirm, palette)
        .fill(fill)
        .on_press_tx(confirm)
        .child(text(question.confirm).font(TYPE_BODY_STRONG).color(ink));
    let frame = col()
        .key(question.key)
        .w(CONFIRM_WIDTH_PT)
        .padding(POPUP_PADDING_PT)
        .gap(SPACE_LG_PT)
        .radius(RADIUS_POPUP_PT)
        .fill(palette.surface_content)
        .border(PILL_OUTLINE_PT, palette.line_subtle)
        .shadow(drop, blur, palette.popup_shadow)
        .role("alertdialog")
        .label(question.title)
        .modal(false, cancel)
        .child(
            text(question.title)
                .font(TYPE_DISPLAY)
                .color(palette.text_primary)
                .role("heading")
                .level(2)
                .wrap(),
        )
        .child(
            text(question.body)
                .font(TYPE_BODY)
                .color(palette.text_secondary)
                .wrap(),
        )
        .child(
            row()
                .w_full()
                .justify_end()
                .gap(SPACE_MD_PT)
                .child(cancel_button)
                .child(confirm_button),
        );
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .padding(SPACE_XXL_PT)
        .fill(palette.scrim)
        .child(frame)
}

fn button(key: &'static str, label: &'static str, palette: &Palette) -> El {
    row()
        .key(key)
        .h(FIELD_HEIGHT_PT)
        .px(SPACE_LG_PT)
        .center()
        .radius(RADIUS_FIELD_PT)
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
}
