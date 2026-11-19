//! The flow's transitions: [`SignIn::step`] and the rules it applies to what
//! was typed.
//!
//! Owns how each event moves the screen and which effects it asks for. The
//! effects are carried out by `requests`; the screens are drawn by `view`.

use rune::TextEdit;
use rune::machine::Effects;

use super::feedback::line_for;
use super::{
    AccountForm, AddressCheck, Created, Discovery, Effect, Event, Failure, Fields, Form, Line,
    NEW_PASSWORD_MIN_CHARS, NEW_USERNAME_MIN_CHARS, PASSWORD_MAX_CHARS, Phase, Probe, ProbeFailure,
    Screen, Server, SignIn, USERNAME_MAX_CHARS, address,
};

impl SignIn {
    /// Takes `event` and returns what the shell must do for it.
    pub(crate) fn step(&mut self, fields: &mut Fields, event: Event) -> Effects<Effect> {
        match event {
            Event::Edited => {
                self.edited(fields);
                Effects::none()
            }
            Event::TogglePassword => {
                fields.password_shown = !fields.password_shown;
                Effects::none()
            }
            Event::Submit => self.submit(fields),
            Event::Open => self.open(fields),
            Event::ChangeServer => self.change_server(fields),
            Event::ShowForm(form) => {
                self.show_form(fields, form);
                Effects::none()
            }
            Event::Probed(probe) => self.probed(fields, probe),
            Event::Discovered(discovery) => self.discovered(discovery),
            Event::Unlocked(result) => {
                self.unlocked(fields, result);
                Effects::none()
            }
            Event::SignedIn(result) => self.signed_in(fields, result),
            Event::Created(created) => self.created(fields, created),
        }
    }

    fn edited(&mut self, fields: &mut Fields) {
        match &mut self.screen {
            Screen::Address { phase, submitted } => {
                fields.address_check = address::check(&fields.address.text);
                *submitted = false;
                if let Phase::Editing(line) = phase {
                    *line = None;
                }
            }
            Screen::Account { phase, .. } => {
                clamp(&mut fields.username, USERNAME_MAX_CHARS);
                clamp(&mut fields.new_username, USERNAME_MAX_CHARS);
                clamp(&mut fields.password, PASSWORD_MAX_CHARS);
                clamp(&mut fields.new_password, PASSWORD_MAX_CHARS);
                clamp(&mut fields.server_password, PASSWORD_MAX_CHARS);
                if let Phase::Editing(line) = phase {
                    *line = None;
                }
            }
        }
    }

    fn submit(&mut self, fields: &mut Fields) -> Effects<Effect> {
        match &mut self.screen {
            Screen::Address { phase, submitted } => {
                if *phase == Phase::Busy {
                    return Effects::none();
                }
                *submitted = true;
                let AddressCheck::Valid { address, .. } = &fields.address_check else {
                    return Effects::none();
                };
                *phase = Phase::Busy;
                Effects::one(Effect::Probe {
                    address: address.clone(),
                })
            }
            Screen::Account { form, phase, .. } => {
                if *phase == Phase::Busy || !can_submit(*form, fields) {
                    return Effects::none();
                }
                *phase = Phase::Busy;
                Effects::one(match form {
                    Form::Unlock { .. } => Effect::Unlock {
                        password: fields.server_password.text.clone(),
                    },
                    Form::Account(AccountForm::SignIn) => Effect::SignIn {
                        username: fields.username.text.trim().to_owned(),
                        password: fields.password.text.clone(),
                    },
                    Form::Account(AccountForm::CreateAccount) => Effect::CreateAccount {
                        username: fields.new_username.text.trim().to_owned(),
                        password: fields.new_password.text.clone(),
                    },
                })
            }
        }
    }

    /// The already-added line's Open: the app if the server has an account,
    /// else its account steps.
    fn open(&mut self, fields: &mut Fields) -> Effects<Effect> {
        let Screen::Address {
            phase: Phase::Editing(Some(Line::AlreadyAdded(added))),
            ..
        } = &self.screen
        else {
            return Effects::none();
        };
        if added.has_account {
            return Effects::one(Effect::OpenServer(added.id));
        }
        let added = added.as_ref().clone();
        let server = Server {
            id: added.id,
            name: added.name,
            address: added.address,
            icon: None,
            unlocked: false,
            renewing: None,
        };
        let mut effects = self.enter_account(fields, server, added.public);
        if let Some(icon) = added.icon {
            effects.push(Effect::LoadIcon(icon));
        }
        effects
    }

    fn change_server(&mut self, fields: &mut Fields) -> Effects<Effect> {
        let Screen::Account { server, phase, .. } = &self.screen else {
            return Effects::none();
        };
        // The same account signs in again: there is no other server to pick.
        if server.renewing.is_some() || *phase == Phase::Busy {
            return Effects::none();
        }
        let id = server.id;
        self.auth = None;
        self.screen = Screen::Address {
            phase: Phase::Editing(None),
            submitted: false,
        };
        clear_passwords(fields);
        fields.address.focused = true;
        Effects::one(Effect::RemoveServer(id))
    }

    fn show_form(&mut self, fields: &mut Fields, wanted: AccountForm) {
        let Screen::Account {
            server,
            form,
            phase,
        } = &mut self.screen
        else {
            return;
        };
        let renewing = server.renewing.is_some();
        if *phase == Phase::Busy || renewing || !matches!(form, Form::Account(_)) {
            return;
        }
        *form = Form::Account(wanted);
        *phase = Phase::Editing(None);
        focus_form(fields, *form);
    }

    fn probed(&mut self, fields: &mut Fields, probe: Probe) -> Effects<Effect> {
        let Screen::Address { phase, .. } = &mut self.screen else {
            return Effects::none();
        };
        match probe {
            Probe::Failed(failure) => {
                *phase = Phase::Editing(Some(match failure {
                    ProbeFailure::Unreachable => Line::Unreachable,
                    ProbeFailure::NotSetUp => Line::NotSetUp,
                    ProbeFailure::NotPapo => Line::NotPapo,
                }));
                // The field took no input while busy; it gets the keyboard back.
                fields.address.focused = true;
                Effects::none()
            }
            Probe::AlreadyAdded(added) => {
                *phase = Phase::Editing(Some(Line::AlreadyAdded(Box::new(added))));
                fields.address.focused = true;
                Effects::none()
            }
            Probe::Added { id, discovery } => {
                let address = match &fields.address_check {
                    AddressCheck::Valid { address, .. } => address::shown(address).to_owned(),
                    AddressCheck::Empty | AddressCheck::Invalid(_) => String::new(),
                };
                let server = Server {
                    id,
                    name: discovery.name.unwrap_or_else(|| address.clone()),
                    address,
                    icon: None,
                    unlocked: false,
                    renewing: None,
                };
                let mut effects = self.enter_account(fields, server, discovery.public);
                if let Some(icon) = discovery.icon {
                    effects.push(Effect::LoadIcon(icon));
                }
                effects
            }
        }
    }

    /// Moves to `server`'s account steps: its password first unless it is
    /// public.
    fn enter_account(
        &mut self,
        fields: &mut Fields,
        server: Server,
        public: bool,
    ) -> Effects<Effect> {
        let form = if public {
            Form::Account(AccountForm::SignIn)
        } else {
            Form::Unlock {
                then: AccountForm::SignIn,
                lapsed: false,
            }
        };
        self.auth = None;
        self.screen = Screen::Account {
            server,
            form,
            phase: Phase::Editing(None),
        };
        focus_form(fields, form);
        Effects::none()
    }

    fn discovered(&mut self, discovery: Option<Discovery>) -> Effects<Effect> {
        let Screen::Account { server, .. } = &mut self.screen else {
            return Effects::none();
        };
        let Some(discovery) = discovery else {
            return Effects::none();
        };
        if let Some(name) = discovery.name {
            server.name = name;
        }
        discovery
            .icon
            .map_or_else(Effects::none, |icon| Effects::one(Effect::LoadIcon(icon)))
    }

    fn unlocked(&mut self, fields: &mut Fields, result: Result<(), Failure>) {
        let Screen::Account {
            server,
            form,
            phase,
        } = &mut self.screen
        else {
            return;
        };
        let Form::Unlock { then, .. } = *form else {
            return;
        };
        match result {
            Ok(()) => {
                server.unlocked = true;
                *form = Form::Account(then);
                *phase = Phase::Editing(None);
                fields.server_password.clear();
                focus_form(fields, *form);
            }
            Err(failure) => {
                let line = match failure {
                    Failure::WrongCredentials => Line::WrongServerPassword,
                    other => line_for(other),
                };
                *phase = Phase::Editing(Some(line));
                focus_form(fields, *form);
            }
        }
    }

    fn signed_in(&mut self, fields: &mut Fields, result: Result<(), Failure>) -> Effects<Effect> {
        let Screen::Account { server, .. } = &self.screen else {
            return Effects::none();
        };
        match result {
            Ok(()) => Effects::one(Effect::Finish {
                server: server.name.clone(),
            }),
            Err(failure) => {
                self.failed(fields, AccountForm::SignIn, failure);
                Effects::none()
            }
        }
    }

    fn created(&mut self, fields: &mut Fields, created: Created) -> Effects<Effect> {
        let Screen::Account { server, form, .. } = &mut self.screen else {
            return Effects::none();
        };
        match created {
            Created::SignedIn => Effects::one(Effect::Finish {
                server: server.name.clone(),
            }),
            Created::NotRegistered(failure) => {
                self.failed(fields, AccountForm::CreateAccount, failure);
                Effects::none()
            }
            Created::NotSignedIn(failure) => {
                // The account exists now: signing in is what is left to do.
                *form = Form::Account(AccountForm::SignIn);
                fields
                    .username
                    .replace_text(fields.new_username.text.trim());
                fields
                    .password
                    .replace_text(fields.new_password.text.clone());
                self.failed(fields, AccountForm::SignIn, failure);
                Effects::none()
            }
        }
    }

    /// Shows why `form`'s request failed; a server that wants its password
    /// goes back to it, warning only when an earlier unlock lapsed.
    fn failed(&mut self, fields: &mut Fields, form_failed: AccountForm, failure: Failure) {
        let Screen::Account {
            server,
            form,
            phase,
        } = &mut self.screen
        else {
            return;
        };
        if failure == Failure::AccessRequired {
            *form = Form::Unlock {
                then: form_failed,
                lapsed: server.unlocked,
            };
            *phase = Phase::Editing(None);
            focus_form(fields, *form);
            return;
        }
        *phase = Phase::Editing(Some(line_for(failure)));
        // The fields took no input while busy; the form gets the keyboard back.
        focus_form(fields, *form);
    }
}

/// Whether `form`'s fields hold enough to send.
pub(crate) fn can_submit(form: Form, fields: &Fields) -> bool {
    match form {
        Form::Unlock { .. } => !fields.server_password.text.is_empty(),
        Form::Account(AccountForm::SignIn) => {
            !fields.username.text.trim().is_empty() && !fields.password.text.is_empty()
        }
        Form::Account(AccountForm::CreateAccount) => {
            fields.new_username.text.trim().chars().count() >= NEW_USERNAME_MIN_CHARS
                && password_rules_met(&fields.new_password.text)
        }
    }
}

/// The test backend's password rules: eight characters or more, an uppercase
/// letter and a symbol.
pub(crate) fn password_rules_met(password: &str) -> bool {
    password.chars().count() >= NEW_PASSWORD_MIN_CHARS
        && password.chars().any(char::is_uppercase)
        && password
            .chars()
            .any(|c| !c.is_alphanumeric() && !c.is_whitespace())
}

/// Focuses the first field `form` asks for.
fn focus_form(fields: &mut Fields, form: Form) {
    match form {
        Form::Unlock { .. } => fields.server_password.focused = true,
        Form::Account(AccountForm::SignIn) => {
            if fields.username.text.is_empty() {
                fields.username.focused = true;
            } else {
                fields.password.focused = true;
            }
        }
        Form::Account(AccountForm::CreateAccount) => fields.new_username.focused = true,
    }
}

fn clear_passwords(fields: &mut Fields) {
    fields.server_password.clear();
    fields.password.clear();
    fields.new_password.clear();
}

/// Cuts `edit` to `max_chars` characters when an edit or a paste ran past it.
fn clamp(edit: &mut TextEdit, max_chars: usize) {
    if let Some((cut, _)) = edit.text.char_indices().nth(max_chars) {
        let kept = edit.text[..cut].to_owned();
        edit.replace_text(kept);
    }
}
