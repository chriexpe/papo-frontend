//! The confirmation popup (`docs/design.md` §10, Confirmations): a title
//! that asks over the dimmed app, what will be lost, then a foot row with
//! an optional note on the left and Cancel, the default, and the action on
//! the right. Cancel has the focus, and Escape and Return cancel. Removing
//! a server, opening a link, sending a message again and deleting one ask
//! through it.
//!
//! Owns the popup's layout and keys; what is asked and what each button
//! does is the caller's.

use rune::{El, Tx, col, el, row, text};

use crate::design::{
    CONFIRM_WIDTH_PT, FIELD_HEIGHT_PT, FIELD_OUTLINE_PT, FOCUS_RING_PT, Look, PILL_OUTLINE_PT,
    POPUP_PADDING_PT, POPUP_SHADOW_PT, RADIUS_FIELD, RADIUS_POPUP, SPACE_LG, SPACE_MD, SPACE_XXL,
    TYPE_BODY, TYPE_BODY_STRONG, TYPE_DISPLAY, TYPE_SMALL,
};
use crate::shell::Shell;

/// What a confirmation asks, and its buttons' keys and words.
pub(crate) struct Question<'a> {
    /// The popup's element key.
    pub(crate) key: &'static str,
    pub(crate) title: &'a str,
    /// What will be lost, in words.
    pub(crate) body: Option<&'a str>,
    /// What will be lost, shown: a message as the conversation draws it.
    pub(crate) shown: Option<El>,
    /// A note on the foot row's left, wrapping, never a line of its own:
    /// words (`note`) or a control, such as the box that trusts a link's
    /// host.
    pub(crate) note: Option<El>,
    pub(crate) cancel_key: &'static str,
    pub(crate) cancel: &'static str,
    pub(crate) confirm_key: &'static str,
    pub(crate) confirm: &'static str,
    /// The action destroys something: drawn in the danger colour, else the
    /// accent.
    pub(crate) danger: bool,
}

/// `question` over the dimmed app, with the keyboard on Cancel. Escape,
/// Return and Cancel call `cancel`; only the action's button calls
/// `confirm`.
pub(crate) fn confirm(
    question: Question<'_>,
    cancel: fn(&mut Shell),
    confirm: fn(&mut Shell, &mut Tx),
    look: &Look,
) -> El {
    let (drop, blur) = POPUP_SHADOW_PT;
    let cancel_button = button(question.cancel_key, question.cancel, look)
        .border(FIELD_OUTLINE_PT, look.palette.line_strong)
        .on_press(cancel)
        // Return cancels: the action is never the default.
        .default_button()
        .autofocus()
        .child(
            text(question.cancel)
                .font(look.font(TYPE_BODY_STRONG))
                .color(look.palette.text_primary),
        );
    let (fill, ink) = if question.danger {
        (look.palette.danger, look.palette.danger_on)
    } else {
        (look.palette.accent, look.palette.accent_on)
    };
    let confirm_button = button(question.confirm_key, question.confirm, look)
        .fill(fill)
        .on_press_tx(confirm)
        .child(
            text(question.confirm)
                .font(look.font(TYPE_BODY_STRONG))
                .color(ink),
        );
    let frame = col()
        .key(question.key)
        .w(CONFIRM_WIDTH_PT)
        .padding(POPUP_PADDING_PT)
        .gap(look.space(SPACE_LG))
        .radius(look.radius(RADIUS_POPUP))
        .fill(look.palette.surface_content)
        .border(PILL_OUTLINE_PT, look.palette.line_subtle)
        .shadow(drop, blur, look.palette.popup_shadow)
        .role("alertdialog")
        .label(question.title)
        .modal(false, cancel)
        .child(
            text(question.title)
                .font(look.font(TYPE_DISPLAY))
                .color(look.palette.text_primary)
                .role("heading")
                .level(2)
                .wrap(),
        )
        .children(question.body.map(|body| {
            text(body)
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_secondary)
                .wrap()
        }))
        .children(question.shown)
        .child(
            row()
                .w_full()
                .items_center()
                .gap(look.space(SPACE_MD))
                .child(
                    col()
                        .key("confirm-note")
                        .grow(1.0)
                        .min_w(0.0)
                        .children(question.note),
                )
                .child(cancel_button)
                .child(confirm_button),
        );
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .padding(look.space(SPACE_XXL))
        .fill(look.palette.scrim)
        .child(frame)
}

/// A note in words, for a question's foot row.
pub(crate) fn note(words: &str, look: &Look) -> El {
    text(words)
        .font(look.font(TYPE_SMALL))
        .color(look.palette.text_muted)
        .wrap()
}

fn button(key: &'static str, label: &'static str, look: &Look) -> El {
    row()
        .key(key)
        .h(FIELD_HEIGHT_PT)
        .px(look.space(SPACE_LG))
        .center()
        .radius(look.radius(RADIUS_FIELD))
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
}
