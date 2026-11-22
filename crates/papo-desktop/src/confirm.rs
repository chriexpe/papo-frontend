//! The confirmation popup (`docs/design.md` §10, Confirmations): a title
//! that asks over the dimmed app, what will be lost, then a foot row with
//! an optional note on the left and Cancel, the default, and the action on
//! the right. Removing a server, sending a message again and deleting one
//! ask through it.
//!
//! Owns the popup's layout and keys; what is asked and what each button
//! does is the caller's.

use rune::{El, Tx, col, el, row, text};

use crate::design::{
    CONFIRM_WIDTH_PT, FIELD_HEIGHT_PT, FIELD_OUTLINE_PT, FOCUS_RING_PT, PILL_OUTLINE_PT,
    POPUP_PADDING_PT, POPUP_SHADOW_PT, Palette, RADIUS_FIELD_PT, RADIUS_POPUP_PT, SPACE_LG_PT,
    SPACE_MD_PT, SPACE_XXL_PT, TYPE_BODY, TYPE_BODY_STRONG, TYPE_DISPLAY, TYPE_SMALL,
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
    /// A note on the foot row's left, wrapping, never a line of its own.
    pub(crate) note: Option<&'a str>,
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
    question: Question<'_>,
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
    let frame =
        col()
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
            .children(question.body.map(|body| {
                text(body)
                    .font(TYPE_BODY)
                    .color(palette.text_secondary)
                    .wrap()
            }))
            .children(question.shown)
            .child(
                row()
                    .w_full()
                    .items_center()
                    .gap(SPACE_MD_PT)
                    .child(col().grow(1.0).min_w(0.0).children(
                        question.note.map(|note| {
                            text(note).font(TYPE_SMALL).color(palette.text_muted).wrap()
                        }),
                    ))
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
