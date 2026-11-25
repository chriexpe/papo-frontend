//! The editor's dialog (`docs/design.md` §14.10): the header with the title
//! and, for an attachment, the name field (its extension fixed and muted;
//! the random name that will be sent while Random name is on) and reroll;
//! the stage or, for a GIF or a file, a preview; the Random name switch,
//! and Anonymize where the SDK can clean the file; the foot
//! with what will be sent and Cancel and Save. Over the dimmed app; Esc
//! cancels, Ctrl+Enter saves.
//!
//! Pure: drawn from the open editor; input goes to the shell.

use rune::{El, Icon, Image, KeyEvent, SurfaceFit, col, el, icon, row, text, text_input};

use super::stage::{controls, stage};
use super::state::{DIALOG_WIDTH_PT, Editor, Use};
use crate::attaching::extension_label;
use crate::composer::tray::TileKind;
use crate::design::{
    EDITOR_PREVIEW_HEIGHT_PT, EDITOR_STAGE, EXTENSION_BLOCK_PT, EXTENSION_BLOCK_RADIUS,
    FIELD_HEIGHT_PT, FOCUS_RING_PT, Look, PILL_OUTLINE_PT, POPUP_SHADOW_PT, RADIUS_CARD,
    RADIUS_CONTROL, RADIUS_PILL, SPACE_LG, SPACE_MD, SPACE_SM, SPACE_XL, SPACE_XXL, Surfaces,
    TYPE_BODY, TYPE_CAPTION, TYPE_SMALL, TYPE_TITLE,
};
use crate::icons::{ANONYMOUS, RENAMED, SHUFFLE};
use crate::settings::switch;
use crate::shell::Shell;
use crate::strings;

/// The element key of the dialog.
pub(crate) const EDITOR_KEY: &str = "editor";
/// The element key of the name field.
pub(crate) const EDITOR_NAME_KEY: &str = "editor-name";

/// What the dialog shows besides the editor's state.
pub(crate) struct Shown<'a> {
    pub(crate) editor: &'a Editor,
    /// The picture, decoded for the stage or preview.
    pub(crate) picture: Option<&'a Image>,
    /// For an attachment: what kind of tile it is.
    pub(crate) kind: TileKind,
    /// The file's name as it is now (the file card's line).
    pub(crate) name: &'a str,
    /// Save waits: another account call runs (a profile picture).
    pub(crate) held: bool,
    /// The file's size, when known (a file card's note).
    pub(crate) size_bytes: Option<u64>,
}

/// The dialog over the dimmed app.
pub(crate) fn dialog(shown: &Shown<'_>, look: &Look) -> El {
    let editor = shown.editor;
    let title = match editor.use_ {
        Use::Attachment => strings::editor_attachment(),
        Use::Avatar => strings::editor_avatar(),
        Use::Banner => strings::editor_banner(),
    };
    let (drop, blur) = POPUP_SHADOW_PT;
    let body = match stage(editor, shown.picture, look) {
        Some(stage) => col().w_full().child(stage).children(controls(editor, look)),
        None => preview(shown, look),
    };
    let frame = col()
        .key(EDITOR_KEY)
        .w(DIALOG_WIDTH_PT)
        .radius(look.radius(RADIUS_PILL))
        .clip_rounded(look.radius(RADIUS_PILL))
        .pill_surface(look)
        .shadow(drop, blur, look.palette.popup_shadow)
        .role("dialog")
        .label(title)
        .modal(false, Shell::editor_cancel)
        .on_key(|shell: &mut Shell, key: &KeyEvent, tx| shell.editor_key(key, tx))
        .child(header(editor, title, look))
        .child(body)
        .children(options(editor, look))
        .child(foot(shown, look));
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .padding(look.space(SPACE_XXL))
        .fill(look.palette.scrim)
        .child(frame)
}

/// The title and, for an attachment, the name field and Random name.
fn header(editor: &Editor, title: &str, look: &Look) -> El {
    let header = row()
        .w_full()
        .pl(look.space(SPACE_XL) + look.space(SPACE_SM))
        .pr(look.space(SPACE_LG) + 2.0)
        .py(look.space(SPACE_LG) + 2.0)
        .gap(look.space(SPACE_MD) + 2.0)
        .items_center()
        .child(
            text(title)
                .font(look.font(TYPE_TITLE))
                .color(look.palette.text_primary)
                .role("heading")
                .level(2),
        );
    if editor.use_ != Use::Attachment {
        return header;
    }
    let field: El = if editor.random_name {
        // The random name that will be sent; reroll draws another.
        text(editor.name.text.clone())
            .key(EDITOR_NAME_KEY)
            .font(look.font(TYPE_BODY))
            .color(look.palette.text_primary)
            .max_lines(1)
            .grow(1.0)
            .min_w(0.0)
    } else {
        text_input(&editor.name, look.font(TYPE_BODY))
            .key(EDITOR_NAME_KEY)
            .label(strings::editor_file_name())
            .grow(1.0)
            .color(look.palette.text_primary)
            .caret(look.palette.accent, look.palette.text_selection)
            .single_line()
            .on_edit(Shell::editor_name_mut)
            // Ctrl+Enter saves from the field too.
            .on_input_key(|shell: &mut Shell, key: &KeyEvent, tx| shell.editor_key(key, tx))
            .on_input_change(|shell: &mut Shell, _tx| shell.editor_name_changed())
            .autofocus()
    };
    let name = row()
        .grow(1.0)
        .min_w(0.0)
        .h(FIELD_HEIGHT_PT)
        .pl(look.space(SPACE_MD) + 2.0)
        .pr(look.space(SPACE_SM))
        .items_center()
        .radius(look.radius(RADIUS_CONTROL))
        .fill(look.palette.surface_content)
        .border(PILL_OUTLINE_PT, look.palette.line_pill)
        .child(field)
        .child(
            text(&editor.extension)
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_muted),
        );
    let mut reroll = row()
        .key("editor-reroll")
        .size(FIELD_HEIGHT_PT, FIELD_HEIGHT_PT)
        .center()
        .radius(look.radius(RADIUS_CONTROL))
        .hover(look.palette.surface_hover)
        .role("button")
        .label(strings::editor_reroll())
        .help(strings::editor_reroll())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press(Shell::editor_reroll)
        .child(icon(&SHUFFLE, 18.0).color(look.palette.text_secondary));
    if editor.random_name {
        // No field to type in: the keyboard starts on reroll.
        reroll = reroll.autofocus();
    }
    header.child(name).child(reroll)
}

/// A GIF's preview or a file's card, where an image has its stage.
fn preview(shown: &Shown<'_>, look: &Look) -> El {
    // A video shows its file card too: its Anonymize and preview come with the SDK's video
    // cleaning.
    if matches!(
        shown.kind,
        TileKind::File | TileKind::Video | TileKind::Audio
    ) {
        let card = row()
            .w_full()
            .px(look.space(SPACE_MD) + 2.0)
            .py(look.space(SPACE_SM) + 2.0)
            .gap(look.space(SPACE_MD) + 2.0)
            .items_center()
            .radius(look.radius(RADIUS_CARD))
            .fill(look.palette.surface_sidebar)
            .border(PILL_OUTLINE_PT, look.palette.line_subtle)
            .child(
                el().size(EXTENSION_BLOCK_PT, EXTENSION_BLOCK_PT)
                    .center()
                    .radius(look.radius(EXTENSION_BLOCK_RADIUS))
                    .fill(look.palette.surface_code)
                    .border(PILL_OUTLINE_PT, look.palette.line_subtle)
                    .child(
                        text(extension_label(shown.name))
                            .font(look.font(crate::design::TYPE_EXTENSION))
                            .color(look.palette.text_secondary),
                    ),
            )
            .child(
                text(shown.name)
                    .font(look.font(TYPE_BODY))
                    .color(look.palette.text_primary)
                    .truncate_middle(),
            );
        return el().w_full().px(look.space(SPACE_XXL)).child(card);
    }
    let mut picture = el()
        .w_full()
        .h(EDITOR_PREVIEW_HEIGHT_PT)
        .radius(look.radius(RADIUS_CARD))
        .clip_rounded(look.radius(RADIUS_CARD))
        .fill(EDITOR_STAGE)
        .a11y_hidden();
    if let Some(image) = shown.picture.filter(|image| image.error().is_none()) {
        picture = picture.pixels_fit(image.pixels(), SurfaceFit::Fit);
    }
    el().w_full().px(look.space(SPACE_XXL)).child(picture)
}

/// Anonymize, where the SDK cleans the file.
fn options(editor: &Editor, look: &Look) -> Option<El> {
    if editor.use_ != Use::Attachment {
        return None;
    }
    let random = OptionRow {
        key: "editor-random-name",
        shape: &RENAMED,
        title: strings::editor_random_name(),
        sub: strings::editor_random_name_sub(),
        on: editor.random_name,
        // Anonymize holds it on.
        disabled: editor.anonymized(),
        pressed: Shell::editor_toggle_random_name,
    };
    let anonymize = editor.anonymize.map(|_| OptionRow {
        key: "editor-anonymize",
        shape: &ANONYMOUS,
        title: strings::editor_anonymize(),
        sub: strings::editor_anonymize_sub(),
        on: editor.anonymized(),
        disabled: false,
        pressed: Shell::editor_toggle_anonymize,
    });
    Some(
        col()
            .w_full()
            .px(look.space(SPACE_XXL))
            .pt(look.space(SPACE_MD) + 2.0)
            .gap(look.space(SPACE_MD))
            .child(random.view(look))
            .children(anonymize.map(|row| row.view(look))),
    )
}

/// One switch under the stage: Random name, Anonymize.
struct OptionRow {
    key: &'static str,
    shape: &'static Icon,
    title: &'static str,
    sub: &'static str,
    on: bool,
    disabled: bool,
    pressed: fn(&mut Shell),
}

impl OptionRow {
    fn view(self, look: &Look) -> El {
        let ink = if self.disabled {
            look.palette.text_muted
        } else {
            look.palette.text_primary
        };
        let mut line = row()
            .key(self.key)
            .w_full()
            .gap(look.space(SPACE_LG))
            .items_center()
            .role("switch")
            .label(self.title)
            .description(self.sub)
            .checked(self.on)
            .disabled(self.disabled)
            .child(
                icon(self.shape, 16.0)
                    .color(look.palette.text_secondary)
                    .a11y_hidden(),
            )
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(text(self.title).font(look.font(TYPE_BODY)).color(ink))
                    .child(
                        text(self.sub)
                            .font(look.font(TYPE_CAPTION))
                            .color(look.palette.text_muted),
                    )
                    .a11y_hidden(),
            )
            .child(switch(self.on, look));
        if !self.disabled {
            line = line
                .focusable(true)
                .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
                .on_press(self.pressed);
        }
        line
    }
}

/// What will be sent, then Cancel and Save.
fn foot(shown: &Shown<'_>, look: &Look) -> El {
    let ready = shown.editor.can_save() && !shown.held;
    let save = crate::confirm::action_button("editor-save", strings::editor_save(), false, look)
        .disabled(!ready)
        .on_press_tx(Shell::editor_save);
    let cancel = crate::confirm::plain_button("editor-cancel", strings::editor_cancel(), look)
        .on_press(Shell::editor_cancel);
    row()
        .w_full()
        .px(look.space(SPACE_XXL))
        .pt(look.space(SPACE_XL))
        .pb(look.space(SPACE_XXL))
        .gap(look.space(SPACE_LG))
        .items_center()
        .child(
            text(note(shown))
                .key("editor-note")
                .font(look.font(TYPE_SMALL).mono())
                .color(look.palette.text_muted)
                .grow(1.0)
                .min_w(0.0)
                .wrap(),
        )
        .child(cancel)
        .child(save)
}

/// The foot's note: "3151 × 2102 · JPEG · 1.8 MB", once edited
/// "3151 × 2102 → 2102 × 2102 · JPEG"; for a fixed use, the size it is
/// saved at.
fn note(shown: &Shown<'_>) -> String {
    let editor = shown.editor;
    // A video shows its file card too: its Anonymize and preview come with the SDK's video
    // cleaning.
    if matches!(
        shown.kind,
        TileKind::File | TileKind::Video | TileKind::Audio
    ) {
        // What goes: its kind and size; a file is never re-encoded.
        let size = shown.size_bytes.map(strings::size).unwrap_or_default();
        return strings::editor_note_file(&extension_label(shown.name), &size);
    }
    let Some(facts) = &editor.facts else {
        return String::new();
    };
    match editor.use_ {
        Use::Avatar => strings::editor_note_fixed(512, 512),
        Use::Banner => strings::editor_note_fixed(1500, 500),
        Use::Attachment => {
            let size = strings::size(facts.size_bytes);
            match editor.crop.as_ref().filter(|crop| !crop.untouched()) {
                Some(crop) => {
                    let (cw, ch) = crop.rect().map_or_else(
                        || {
                            if crop.turns % 2 == 1 {
                                (facts.height, facts.width)
                            } else {
                                (facts.width, facts.height)
                            }
                        },
                        |rect| (rect.width, rect.height),
                    );
                    strings::editor_note_edited(facts.width, facts.height, cw, ch, facts.format)
                }
                None => strings::editor_note(facts.width, facts.height, facts.format, &size),
            }
        }
    }
}
