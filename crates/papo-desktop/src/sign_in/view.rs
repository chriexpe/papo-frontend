//! Draws the server and account screens and the popup that hosts them in the
//! app (`docs/design.md` §7, Layout). Pure: reads the flow and its fields.
//!
//! Owns the layout of each step: a centred column of one width in both hosts,
//! no boxes around the content, one primary button, secondary actions as
//! links. The controls are `controls`'; the window host is the shell's.

use rune::{El, col, el, icon, row, text};

use super::header::{server_header, title};

use crate::design::{
    CROSS_PT, FOCUS_RING_PT, FORM_WIDTH_PT, PILL_OUTLINE_PT, POPUP_CLOSE_INSET_PT, POPUP_CLOSE_PT,
    POPUP_PADDING_PT, POPUP_SHADOW_PT, POPUP_SIZE_PT, Palette, RADIUS_CONTROL_PT, RADIUS_POPUP_PT,
    SPACE_LG_PT, SPACE_SM_PT, SPACE_XL_PT, SPACE_XXL_PT, TYPE_CAPTION,
};
use crate::icons::CROSS;
use crate::shell::Shell;
use crate::strings;

use super::controls::{self, Field};
use super::{
    AccountForm, Event, Fields, Form, Line, Phase, Screen, Server, SignIn, can_submit,
    password_rules_met,
};

/// The element key of the screen's column.
pub(crate) const SCREEN_KEY: &str = "sign-in";

/// The screen: the server step or the account steps, `FORM_WIDTH_PT` wide.
pub(crate) fn screen(flow: &SignIn, fields: &Fields, palette: &Palette) -> El {
    let column = col()
        .key(SCREEN_KEY)
        .w(FORM_WIDTH_PT)
        .max_w(FORM_WIDTH_PT)
        .gap(SPACE_LG_PT);
    match flow.screen() {
        Screen::Address { phase, .. } => address_step(column, flow, fields, phase, palette),
        Screen::Account {
            server,
            form,
            phase,
        } => column
            .gap(SPACE_XL_PT)
            .child(server_header(server, *form, palette))
            .child(account_step(server, *form, phase, fields, palette)),
    }
}

/// The popup over the dimmed app: the screen in a frame, with ✕. Escape and
/// ✕ close it; the app behind takes no input meanwhile.
pub(crate) fn popup(flow: &SignIn, fields: &Fields, palette: &Palette) -> El {
    let (drop, blur) = POPUP_SHADOW_PT;
    let close = row()
        .key("sign-in-close")
        .absolute(
            POPUP_SIZE_PT.0 - POPUP_CLOSE_INSET_PT - POPUP_CLOSE_PT,
            POPUP_CLOSE_INSET_PT,
        )
        .size(POPUP_CLOSE_PT, POPUP_CLOSE_PT)
        .center()
        .radius(RADIUS_CONTROL_PT)
        .role("button")
        .label(strings::close())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, palette.accent)
        .on_press(Shell::close_sign_in)
        .child(icon(&CROSS, CROSS_PT).color(palette.text_secondary));
    let frame = col()
        .key("sign-in-popup")
        .size(POPUP_SIZE_PT.0, POPUP_SIZE_PT.1)
        .center()
        .padding(POPUP_PADDING_PT)
        .radius(RADIUS_POPUP_PT)
        .fill(palette.surface_content)
        .border(PILL_OUTLINE_PT, palette.line_subtle)
        .shadow(drop, blur, palette.popup_shadow)
        .role("dialog")
        .modal(false, Shell::close_sign_in)
        .child(close)
        .child(screen(flow, fields, palette));
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center()
        .padding(SPACE_XXL_PT)
        .fill(palette.scrim)
        .child(frame)
}

fn address_step(
    column: El,
    flow: &SignIn,
    fields: &Fields,
    phase: &Phase,
    palette: &Palette,
) -> El {
    let busy = *phase == Phase::Busy;
    let line = flow.address_line(fields);
    let invalid = matches!(line, Some(Line::Address(_)));
    let field = Field {
        key: "address",
        label: strings::address(),
        edit: &fields.address,
        access: |shell| &mut shell.sign_in_fields_mut().address,
        busy,
        invalid,
    };
    let mut entry = col().w_full().gap(SPACE_SM_PT).child(controls::field(
        &field,
        Some(strings::address_placeholder()),
        palette,
    ));
    if let Some(line) = &line {
        entry = entry.child(controls::feedback(line, palette));
    }
    column
        .child(title(strings::add_server(), palette))
        .child(entry)
        .child(controls::primary_button(
            "connect",
            strings::connect(),
            strings::connecting(),
            busy,
            !matches!(fields.address_check, super::AddressCheck::Empty),
            submit,
            palette,
        ))
}

fn account_step(
    server: &Server,
    form: Form,
    phase: &Phase,
    fields: &Fields,
    palette: &Palette,
) -> El {
    let busy = *phase == Phase::Busy;
    let line = match phase {
        Phase::Editing(line) => line.as_ref(),
        Phase::Busy => None,
    };
    // The field the server rejected shows it in its outline.
    let rejected = line.is_some_and(|line| {
        matches!(
            line,
            Line::WrongServerPassword | Line::WrongCredentials | Line::NameTaken
        )
    });
    let state = FormState {
        fields,
        busy,
        rejected,
        palette,
    };
    let (inputs, button) = match form {
        Form::Unlock { .. } => unlock_inputs(&state),
        Form::Account(AccountForm::SignIn) => sign_in_inputs(&state),
        Form::Account(AccountForm::CreateAccount) => create_account_inputs(&state),
    };
    let mut column = col().w_full().gap(SPACE_LG_PT).children(inputs);
    if let Some(line) = line {
        column = column.child(controls::feedback(line, palette));
    }
    column = column.child(controls::primary_button(
        button.key,
        button.label,
        button.busy_label,
        busy,
        can_submit(form, fields),
        submit,
        palette,
    ));
    let links = links(server, form, palette);
    if links.is_empty() {
        column
    } else {
        column.child(
            row()
                .w_full()
                .justify_center()
                .gap(SPACE_XL_PT)
                .children(links),
        )
    }
}

/// What every account form's inputs are drawn from.
struct FormState<'a> {
    fields: &'a Fields,
    busy: bool,
    /// The server rejected what the form's main field holds.
    rejected: bool,
    palette: &'a Palette,
}

/// A form's primary button: its key and its two labels.
struct Button {
    key: &'static str,
    label: &'static str,
    busy_label: &'static str,
}

fn unlock_inputs(state: &FormState) -> (Vec<El>, Button) {
    let password = controls::password_field(
        &Field {
            key: "server-password",
            label: strings::server_password(),
            edit: &state.fields.server_password,
            access: |shell| &mut shell.sign_in_fields_mut().server_password,
            busy: state.busy,
            invalid: state.rejected,
        },
        state.fields.password_shown,
        toggle_password,
        state.palette,
    );
    let button = Button {
        key: "unlock",
        label: strings::unlock(),
        busy_label: strings::unlocking(),
    };
    (vec![password], button)
}

fn sign_in_inputs(state: &FormState) -> (Vec<El>, Button) {
    let username = controls::field(
        &Field {
            key: "username",
            label: strings::username(),
            edit: &state.fields.username,
            access: |shell| &mut shell.sign_in_fields_mut().username,
            busy: state.busy,
            invalid: false,
        },
        None,
        state.palette,
    );
    let password = controls::password_field(
        &Field {
            key: "password",
            label: strings::password(),
            edit: &state.fields.password,
            access: |shell| &mut shell.sign_in_fields_mut().password,
            busy: state.busy,
            invalid: state.rejected,
        },
        state.fields.password_shown,
        toggle_password,
        state.palette,
    );
    let button = Button {
        key: "sign-in-button",
        label: strings::sign_in(),
        busy_label: strings::signing_in(),
    };
    (vec![username, password], button)
}

fn create_account_inputs(state: &FormState) -> (Vec<El>, Button) {
    let palette = state.palette;
    let username = controls::field(
        &Field {
            key: "new-username",
            label: strings::username(),
            edit: &state.fields.new_username,
            access: |shell| &mut shell.sign_in_fields_mut().new_username,
            busy: state.busy,
            invalid: state.rejected,
        },
        None,
        palette,
    );
    let password = controls::password_field(
        &Field {
            key: "new-password",
            label: strings::password(),
            edit: &state.fields.new_password,
            access: |shell| &mut shell.sign_in_fields_mut().new_password,
            busy: state.busy,
            invalid: false,
        },
        state.fields.password_shown,
        toggle_password,
        palette,
    );
    let hint_color = if password_rules_met(&state.fields.new_password.text) {
        palette.success
    } else {
        palette.text_muted
    };
    let hint = text(strings::password_hint())
        .key("password-hint")
        .font(TYPE_CAPTION)
        .color(hint_color);
    let password = col().w_full().gap(SPACE_SM_PT).child(password).child(hint);
    let button = Button {
        key: "create-account-button",
        label: strings::create_account(),
        busy_label: strings::creating_account(),
    };
    (vec![username, password], button)
}

/// The links under the button. Signing in again hides Create account and
/// Change server: it is the same account on the same server.
fn links(server: &Server, form: Form, palette: &Palette) -> Vec<El> {
    if server.renewing.is_some() {
        return Vec::new();
    }
    match form {
        Form::Unlock { .. } => vec![change_server(palette)],
        Form::Account(AccountForm::SignIn) => vec![
            controls::link(
                "to-create-account",
                strings::create_account(),
                false,
                |shell| {
                    shell.sign_in_event(Event::ShowForm(AccountForm::CreateAccount));
                },
                palette,
            ),
            change_server(palette),
        ],
        Form::Account(AccountForm::CreateAccount) => vec![controls::link(
            "to-sign-in",
            strings::have_account(),
            false,
            |shell| {
                shell.sign_in_event(Event::ShowForm(AccountForm::SignIn));
            },
            palette,
        )],
    }
}

fn change_server(palette: &Palette) -> El {
    controls::link(
        "change-server",
        strings::change_server(),
        true,
        |shell| {
            shell.sign_in_event(Event::ChangeServer);
        },
        palette,
    )
}

fn submit(shell: &mut Shell) {
    shell.sign_in_event(Event::Submit);
}

fn toggle_password(shell: &mut Shell) {
    shell.sign_in_event(Event::TogglePassword);
}
