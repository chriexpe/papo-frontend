//! The text pill (`docs/design.md` §3, Composer; the mock's `.comp`): the
//! field, growing to [`COMPOSER_MAX_LINES`] lines and then scrolling inside,
//! and Send (Save while editing, §11), with the line over it that says the
//! text is too long or a send failed, and the reply or edit bar as its
//! header row (`bars`). Emoji, stickers and GIF join the pill with their pickers (D11).
//!
//! Pure: it reads the composer and hands Enter, Shift+Enter, edits and Send
//! to the shell.

use papo_sdk::Uuid;
use rune::{El, KeyEvent, PillStyle, auto_grow, col, icon, row, text, text_input};

use super::Composer;
use super::bars::bar;
use super::picker::Picker;
use super::tray_ui::TrayUi;
use super::tray_view::tray;
use crate::design::{
    COMPOSER_HEIGHT, COMPOSER_MAX_LINES, FOCUS_RING_PT, Look, PILL_ICON_PT, PILL_OUTLINE_PT,
    RADIUS_COMPOSER, RADIUS_PILL, SEND_BUTTON, SPACE_LG, SPACE_MD, SPACE_SM, Surfaces, TYPE_BODY,
    TYPE_SMALL,
};
use crate::emoji_images::{EmojiKey, EmojiLooks, EmojiPlace, inline_image};
use crate::icons::{CHECK, PAPERCLIP, SEND};
use crate::shell::Shell;
use crate::strings;
use crate::timeline::replies::OriginalText;

/// The element key of the composer's text field.
pub(crate) const COMPOSER_KEY: &str = "composer";

/// The element key of the text pill, which the @ picker grows out of.
pub(crate) const COMPOSER_PILL_KEY: &str = "composer-pill";

/// The element key of the paperclip squircle.
pub(crate) const ATTACH_KEY: &str = "composer-attach";

/// The element key of Save, Send's place while editing.
pub(crate) const SAVE_KEY: &str = "composer-save";

/// The opacity of Send while there is nothing to send.
const SEND_IDLE_OPACITY: f32 = 0.4;

/// What the pill shows besides the composer's own state.
pub(crate) struct PillContext<'a> {
    /// The empty field's prompt: "Message #general", "Message @Lia".
    pub(crate) placeholder: &'a str,
    /// The "Enter to send" setting.
    pub(crate) enter_sends: bool,
    /// A reply's original in words, while one is being written.
    pub(crate) original: Option<&'a OriginalText>,
    /// The account's user.
    pub(crate) me: Uuid,
    /// The tray's keyboard and drag state.
    pub(crate) tray_ui: &'a TrayUi,
    /// What custom emoji draw as.
    pub(crate) emoji: EmojiLooks,
}

/// The pill for the shown channel, with the line over it when there is
/// one and the reply or edit bar on top.
pub(crate) fn pill(composer: &Composer, context: &PillContext<'_>, look: &Look) -> El {
    let PillContext {
        placeholder,
        enter_sends,
        original,
        me,
        tray_ui,
        ref emoji,
    } = *context;
    let Some((key, draft)) = composer.shown() else {
        // Nothing to write in: still the notice pill's anchor, at no height.
        return col().key(COMPOSER_PILL_KEY).w_full();
    };
    let loading = composer.is_loading();
    // The reply's original draws its custom emoji too.
    let bar_emoji = match original {
        Some(OriginalText::Message { emoji: ranges, .. }) => emoji.of(key.account, ranges),
        _ => Vec::new(),
    };
    let emoji = FieldEmoji {
        looks: emoji.clone(),
        account: key.account,
    };
    let field = field(composer, &draft.edit, placeholder, enter_sends, emoji, look);
    let line_pt = look.font(TYPE_BODY).line_px();
    // One line sits level with Send; more lines grow the pill upward.
    let field = col()
        .grow(1.0)
        .min_w(0.0)
        .py(((look.space(SEND_BUTTON) - line_pt) / 2.0).max(0.0))
        .child(auto_grow(field, 1, COMPOSER_MAX_LINES));
    let send_inset_pt = (look.space(COMPOSER_HEIGHT) - look.space(SEND_BUTTON)) / 2.0;
    let editing = draft.mode.editing().is_some();
    let action = if editing {
        save_button(!loading, look)
    } else {
        send_button(composer.can_send(), look)
    };
    let writing = row()
        .w_full()
        .min_h(look.space(COMPOSER_HEIGHT) - PILL_OUTLINE_PT * 2.0)
        .pl(look.space(SPACE_LG))
        .pr(send_inset_pt)
        .py(send_inset_pt)
        .gap(look.space(SPACE_SM))
        .items_end()
        .child(field)
        .child(action);
    let pill = col()
        .key(COMPOSER_PILL_KEY)
        .w_full()
        .radius(look.radius(RADIUS_COMPOSER))
        .pill_surface(look)
        .children(bar(&draft.mode, original, me, &bar_emoji, look))
        // An edit leaves the attachments alone (§11), so they are not shown.
        .children(
            draft
                .mode
                .editing()
                .is_none()
                .then(|| tray(&draft.tray, tray_ui, look))
                .flatten(),
        )
        .child(writing);
    let note = if composer.chars_over() > 0 {
        let over = u64::try_from(composer.chars_over()).unwrap_or(u64::MAX);
        Some(strings::composer_too_long(over))
    } else if composer.send_failed_here() {
        Some(strings::composer_send_failed().to_owned())
    } else {
        None
    };
    let attach = draft.mode.editing().is_none().then(|| paperclip(look));
    let stack = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(look.space(SPACE_SM))
        .children(note.map(|note| {
            text(note)
                .key("composer-note")
                .font(look.font(TYPE_SMALL))
                .color(look.palette.danger)
                .px(look.space(SPACE_LG))
                .role("status")
                .live_region(rune::LiveRegion::Polite)
                .wrap()
        }))
        .child(pill);
    row()
        .key("composer-area")
        // Its own landmark, so F6 reaches the field from the conversation.
        .role("form")
        .label(strings::composer_label())
        .w_full()
        .gap(look.space(SPACE_MD))
        .items_end()
        .children(attach)
        .child(stack)
}

/// The paperclip squircle beside the text pill (§3, §14.1): the system's
/// file chooser. Left out while a message is edited, never greyed (§10).
fn paperclip(look: &Look) -> El {
    let side = look.space(COMPOSER_HEIGHT);
    row()
        .key(ATTACH_KEY)
        .size(side, side)
        .center()
        .radius(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .role("button")
        .label(strings::attach_button())
        .help(strings::attach_button())
        .help_keys([strings::shortcut_attach()])
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(|shell: &mut Shell, tx| shell.attach_pressed(tx))
        .hover_group()
        .child(
            icon(&PAPERCLIP, PILL_ICON_PT)
                .color(look.palette.text_secondary)
                .group_color(look.palette.text_primary),
        )
}

/// The message field: Markdown styled in place, mentions as pills, Enter
/// and Send, pastes of files, and no typing while the draft loads.
/// The field's custom emoji: what they draw as, for the account it writes as.
struct FieldEmoji {
    looks: EmojiLooks,
    account: papo_sdk::AccountId,
}

fn field(
    composer: &Composer,
    edit: &rune::TextEdit,
    placeholder: &str,
    enter_sends: bool,
    emoji: FieldEmoji,
    look: &Look,
) -> El {
    let loading = composer.is_loading();
    let style = composer.field_style(look);
    let (chip, ink) = (look.palette.accent_soft, look.palette.link);
    let plain = look.palette.text_primary;
    let mut field = text_input(edit, look.font(TYPE_BODY))
        .key(COMPOSER_KEY)
        .label(strings::composer_label())
        // A long link or word breaks at the field's edge instead of running
        // past it.
        .wrap_anywhere()
        .grow(1.0)
        .color(look.palette.text_primary)
        .placeholder(placeholder, look.palette.text_muted)
        .caret(look.palette.accent, look.palette.text_selection)
        .spans(style.spans)
        .strikes(style.strikes)
        // A mention's pill is drawn as a message draws one; an emoji's whose image failed is
        // its name as plain text (§9).
        .pill_style(move |id| match super::emoji::of_pill(id) {
            Some(_) => PillStyle::default().bg(rune::Color::TRANSPARENT).fg(plain),
            None => PillStyle::default().bg(chip).fg(ink),
        })
        // An emoji's pill is its image at line height (§16.2, §9 Inline).
        .pill_inline_image({
            let ui = *look;
            let font = look.font(TYPE_BODY);
            move |id| {
                let key = EmojiKey {
                    account: emoji.account,
                    id: super::emoji::of_pill(id)?,
                };
                let place = EmojiPlace {
                    font,
                    jumbo: false,
                    hidden: false,
                };
                inline_image(&emoji.looks.look(key), place, &ui)
            }
        });
    if let Some(row) = composer.picker().and_then(Picker::chosen_key) {
        field = field.active_descendant(row).expanded(true);
    }
    if loading {
        // No `on_edit` while the draft loads: typing waits for it.
        field = field.busy(true);
    } else {
        field = field
            .on_edit(|shell: &mut Shell| shell.composer.field_mut())
            .on_input_change(Shell::composer_edited)
            // With "Enter to send" on, plain Enter submits and Shift+Enter
            // is the editor's own line break.
            .on_submit(Shell::composer_send)
            // Files and pictures pasted become tiles; text stays text.
            .on_input_paste(Shell::composer_paste)
            .on_input_key(move |shell: &mut Shell, key: &KeyEvent, tx| {
                shell.composer_key(key, enter_sends, tx)
            });
    }
    field
}

/// Send's and Save's corner radius: concentric with the pill, whose edge
/// is the action's inset out (`pill`'s `send_inset_pt`).
fn action_radius(look: &Look) -> f32 {
    let inset_pt = (look.space(COMPOSER_HEIGHT) - look.space(SEND_BUTTON)) / 2.0;
    Look::nested_radius(look.radius(RADIUS_COMPOSER), inset_pt)
}

/// Save in Send's place while editing: the check on the accent square,
/// named "Save edit". An emptied edit saves too: it asks to delete instead.
fn save_button(enabled: bool, look: &Look) -> El {
    let button = row()
        .key(SAVE_KEY)
        .size(look.space(SEND_BUTTON), look.space(SEND_BUTTON))
        .center()
        .radius(action_radius(look))
        .fill(look.palette.accent)
        .role("button")
        .label(strings::edit_save())
        .disabled(!enabled)
        .child(icon(&CHECK, PILL_ICON_PT).color(look.palette.accent_on));
    if enabled {
        button
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(Shell::composer_send)
    } else {
        button.opacity(SEND_IDLE_OPACITY)
    }
}

/// Send: the accent square with the arrow, dimmed while there is nothing
/// to send.
fn send_button(enabled: bool, look: &Look) -> El {
    let button = row()
        .key("composer-send")
        .size(look.space(SEND_BUTTON), look.space(SEND_BUTTON))
        .center()
        .radius(action_radius(look))
        .fill(look.palette.accent)
        .role("button")
        .label(strings::composer_send())
        .disabled(!enabled)
        .child(icon(&SEND, PILL_ICON_PT).color(look.palette.accent_on));
    if enabled {
        button
            .focusable(true)
            .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
            .on_press_tx(Shell::composer_send)
    } else {
        button.opacity(SEND_IDLE_OPACITY)
    }
}
