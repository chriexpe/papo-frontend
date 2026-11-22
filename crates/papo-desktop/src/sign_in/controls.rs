//! The form controls the server and account screens are built from: a
//! labelled field, a password field with Show and Hide, the primary button
//! with its busy spinner, a link-styled button and the feedback line.
//!
//! Owns how each control looks and what assistive technology hears; which
//! controls a step shows, and what pressing them does, is the screen's.

use rune::{El, LiveRegion, LoopSpec, TextEdit, TextStyle, col, el, icon, row, text, text_input};

use crate::design::{
    BUTTON_DISABLED_OPACITY, FIELD_BUSY_OPACITY, FIELD_BUTTON_HEIGHT_PT, FIELD_HEIGHT_PT,
    FIELD_OUTLINE_PT, FOCUS_RING_PT, Palette, RADIUS_CONTROL_PT, RADIUS_FIELD_PT, SPACE_LG_PT,
    SPACE_MD_PT, SPACE_SM_PT, SPINNER_PERIOD_S, SPINNER_PT, TYPE_BODY, TYPE_BODY_STRONG,
    TYPE_SMALL, TYPE_SMALL_MEDIUM,
};
use crate::icons::SPINNER;
use crate::shell::Shell;
use crate::strings;

use super::{Line, LineAction, Tone};

/// Picks a field's text out of the state, for its edits.
pub(crate) type Access = fn(&mut Shell) -> &mut TextEdit;

/// A text field as a form shows it.
pub(crate) struct Field<'a> {
    pub(crate) key: &'static str,
    pub(crate) label: &'static str,
    pub(crate) edit: &'a TextEdit,
    pub(crate) access: Access,
    /// A request runs: the field keeps its text and takes no input.
    pub(crate) busy: bool,
    /// Its outline shows the danger colour.
    pub(crate) invalid: bool,
}

/// The label above a field.
pub(crate) fn label(label: &'static str, palette: &Palette) -> El {
    text(label)
        .font(TYPE_SMALL_MEDIUM)
        .color(palette.text_secondary)
        .a11y_hidden()
}

/// A labelled field: the label above it, and the field's own outline.
pub(crate) fn field(field: &Field, placeholder: Option<&'static str>, palette: &Palette) -> El {
    let mut input = input(field, palette);
    if let Some(placeholder) = placeholder {
        input = input.placeholder(placeholder, palette.text_muted);
    }
    col()
        .w_full()
        .gap(SPACE_MD_PT)
        .child(label(field.label, palette))
        .child(frame(field, palette).child(input))
}

/// A password field: the label, then the field with Show or Hide at its end.
/// `toggle` flips whether every password field shows its characters.
pub(crate) fn password_field(
    field: &Field,
    shown: bool,
    toggle: fn(&mut Shell),
    palette: &Palette,
) -> El {
    let (word, spoken) = if shown {
        (strings::hide(), strings::hide_password())
    } else {
        (strings::show(), strings::show_password())
    };
    let reveal = row()
        .key(format!("{}-reveal", field.key))
        .h(FIELD_BUTTON_HEIGHT_PT)
        .px(SPACE_MD_PT)
        .center_y()
        .radius(RADIUS_CONTROL_PT)
        .role("button")
        .label(spoken)
        .focusable(!field.busy)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .on_press(toggle)
        .child(
            text(word)
                .font(TYPE_SMALL_MEDIUM)
                .color(palette.text_secondary),
        );
    let input = input(field, palette).secure().revealed(shown);
    col()
        .w_full()
        .gap(SPACE_MD_PT)
        .child(label(field.label, palette))
        .child(
            frame(field, palette)
                .pr(SPACE_SM_PT)
                .gap(SPACE_SM_PT)
                .child(input)
                .child(reveal),
        )
}

fn input(field: &Field, palette: &Palette) -> El {
    let input = text_input(field.edit, TYPE_BODY)
        .key(field.key)
        .label(field.label)
        .single_line()
        .grow(1.0)
        .color(palette.text_primary)
        .caret(palette.accent, palette.text_selection)
        .invalid(field.invalid)
        .on_edit(field.access)
        .on_input_change(|shell: &mut Shell, _tx| {
            shell.sign_in_event(super::Event::Edited);
        });
    if field.busy { input.inert() } else { input }
}

/// The outlined box a field sits in.
fn frame(field: &Field, palette: &Palette) -> El {
    let outline = if field.invalid {
        palette.danger
    } else if field.edit.focused {
        palette.accent
    } else {
        palette.line_strong
    };
    let frame = row()
        .w_full()
        .h(FIELD_HEIGHT_PT)
        .px(SPACE_LG_PT)
        .center_y()
        .radius(RADIUS_FIELD_PT)
        .fill(palette.surface_code)
        .border(FIELD_OUTLINE_PT, outline);
    if field.busy {
        frame.opacity(FIELD_BUSY_OPACITY)
    } else {
        frame
    }
}

/// The step's one primary button. While `busy` it shows `busy_label` and a
/// spinner; it presses only when `ready`. Return presses it from anywhere in
/// the form.
pub(crate) fn primary_button(
    key: &'static str,
    label: &'static str,
    busy_label: &'static str,
    busy: bool,
    ready: bool,
    press: fn(&mut Shell),
    palette: &Palette,
) -> El {
    let enabled = ready && !busy;
    let mut button = row()
        .key(key)
        .w_full()
        .h(FIELD_HEIGHT_PT)
        .center()
        .gap(SPACE_MD_PT)
        .radius(RADIUS_FIELD_PT)
        .fill(palette.accent)
        .role("button")
        .label(if busy { busy_label } else { label })
        .disabled(!enabled)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring);
    if enabled {
        button = button.focusable(true).on_press(press).default_button();
    } else {
        button = button.opacity(if busy { 1.0 } else { BUTTON_DISABLED_OPACITY });
    }
    if busy {
        button = button.child(
            icon(&SPINNER, SPINNER_PT)
                .color(palette.accent_on)
                .looping(LoopSpec::rotation(SPINNER_PERIOD_S))
                .a11y_hidden(),
        );
    }
    button.child(
        text(if busy { busy_label } else { label })
            .font(TYPE_BODY_STRONG)
            .color(palette.accent_on),
    )
}

/// A secondary action drawn as a link: in the link colour, or in the
/// secondary text colour when `dim`.
pub(crate) fn link(
    key: &'static str,
    label: &'static str,
    dim: bool,
    press: fn(&mut Shell),
    palette: &Palette,
) -> El {
    link_styled(key, label, dim, TYPE_SMALL_MEDIUM, press, palette)
}

fn link_styled(
    key: &'static str,
    label: &'static str,
    dim: bool,
    style: TextStyle,
    press: fn(&mut Shell),
    palette: &Palette,
) -> El {
    let color = if dim {
        palette.text_secondary
    } else {
        palette.link
    };
    el().key(key)
        .role("button")
        .label(label)
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.focus_ring)
        .on_press(press)
        .child(text(label).font(style).color(color))
}

/// The feedback line, in its tone's colour, with its action. Errors are
/// announced at once; warnings when assistive technology is idle.
pub(crate) fn feedback(line: &Line, palette: &Palette) -> El {
    let (color, announce, role) = match line.tone() {
        Tone::Danger => (palette.danger, LiveRegion::Assertive, "alert"),
        Tone::Warning => (palette.warning, LiveRegion::Polite, "status"),
    };
    let words = line_text(line);
    let mut line_el = row()
        .key("feedback")
        .w_full()
        .gap(SPACE_SM_PT)
        .flex_wrap()
        .role(role)
        .live_region(announce)
        .child(text(words).font(TYPE_SMALL).color(color).wrap());
    if let Some(action) = line.action() {
        let (key, label, press): (_, _, fn(&mut Shell)) = match action {
            LineAction::TryAgain => ("feedback-try-again", strings::try_again(), |shell| {
                shell.sign_in_event(super::Event::Submit);
            }),
            LineAction::Open => ("feedback-open", strings::open(), |shell| {
                shell.sign_in_event(super::Event::Open);
            }),
        };
        line_el = line_el.child(link_styled(key, label, false, TYPE_SMALL, press, palette));
    }
    line_el
}

/// The words of `line`, in the current language.
pub(crate) fn line_text(line: &Line) -> String {
    use super::AddressProblem;
    let words = match line {
        Line::Address(AddressProblem::Scheme) => strings::error_scheme(),
        Line::Address(AddressProblem::Userinfo) => strings::error_userinfo(),
        Line::Address(AddressProblem::Query) => strings::error_query(),
        Line::Address(AddressProblem::Fragment) => strings::error_fragment(),
        Line::Address(AddressProblem::Malformed) => strings::error_invalid(),
        Line::Unencrypted => strings::error_unencrypted(),
        Line::Unreachable => strings::error_unreachable(),
        Line::NotSetUp => strings::error_not_set_up(),
        Line::NotPapo => strings::error_not_papo(),
        Line::AlreadyAdded(added) => return strings::error_already_added(&added.name),
        Line::WrongServerPassword => strings::error_wrong_server_password(),
        Line::WrongCredentials => strings::error_wrong_credentials(),
        Line::Banned => strings::error_banned(),
        Line::RateLimited => strings::error_rate_limited(),
        Line::NoResponse => strings::error_no_response(),
        Line::NameTaken => strings::error_name_taken(),
        Line::Keyring => strings::error_keyring(),
        Line::Failed => strings::error_failed(),
    };
    words.to_owned()
}
