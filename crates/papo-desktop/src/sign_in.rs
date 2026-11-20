//! The server and account screens of `docs/design.md` §7: the address, the
//! server's password, sign in and create account, and signing in again after
//! a session expired. One screen, shown by one of two hosts: the whole window
//! on first run, a popup over the app otherwise.
//!
//! Owns the flow as one state machine: [`SignIn::step`] takes an [`Event`]
//! (a press, a submit, an SDK answer) and returns the [`Effect`]s to carry
//! out, which `requests` does. The SDK owns the server list, accounts and
//! credentials; this keeps only which screen shows, what the person typed,
//! and the feedback line. Recovery and the forced new password wait for their
//! SDK signals and are not here.

mod address;
mod controls;
mod events;
mod feedback;
mod header;
mod requests;
#[cfg(test)]
mod tests;
mod transitions;
mod view;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use papo_sdk::{AccountId, AuthService, ServerId};
use rune::{Image, TextEdit};

pub(crate) use address::{AddressCheck, AddressProblem};
pub(crate) use events::{Created, Discovery, Effect, Event, Failure, Probe, ProbeFailure};
pub(crate) use feedback::{AlreadyAdded, Line, LineAction, SavedAccounts, Tone};
pub(crate) use requests::{remove_left_server, run};
pub(crate) use transitions::{can_submit, password_rules_met};
pub(crate) use view::{CROSS, popup, screen};

use crate::account_watch::{Credentials, SignedOut};

/// The longest username the account steps take, in characters.
pub(crate) const USERNAME_MAX_CHARS: usize = 16;
/// The longest password the account steps take, in characters.
pub(crate) const PASSWORD_MAX_CHARS: usize = 64;
/// The shortest username Create account takes, in characters.
pub(crate) const NEW_USERNAME_MIN_CHARS: usize = 3;
/// The shortest password Create account takes, in characters. The rules are
/// the test backend's (`docs/design.md` §7, Open).
pub(crate) const NEW_PASSWORD_MIN_CHARS: usize = 8;

/// Which host shows the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Host {
    /// First run: the screen fills the window, with no way out but signing in.
    Window,
    /// In the app: a popup over the dimmed app, closed with ✕ or Escape.
    Popup,
}

/// The flow's state while its screen is open.
pub(crate) struct SignIn {
    host: Host,
    screen: Screen,
    /// The SDK's sign-in handle for the server on screen, once a request has
    /// fetched it. An unlock leaves the server's access in it, so the sign-in
    /// that follows must use the same one.
    pub(crate) auth: Option<AuthService>,
    /// Set once the person closes the screens, for a probe still running.
    abandoned: Arc<AtomicBool>,
}

/// What the person types, kept while the flow moves between steps. Dropping
/// it wipes the passwords.
pub(crate) struct Fields {
    pub(crate) address: TextEdit,
    pub(crate) server_password: TextEdit,
    pub(crate) username: TextEdit,
    pub(crate) password: TextEdit,
    pub(crate) new_username: TextEdit,
    pub(crate) new_password: TextEdit,
    /// Whether the password fields show their characters.
    pub(crate) password_shown: bool,
    /// What the address field holds, checked on every edit.
    pub(crate) address_check: AddressCheck,
}

impl Default for Fields {
    fn default() -> Self {
        Fields {
            address: TextEdit::new(""),
            server_password: TextEdit::new("").secure(),
            username: TextEdit::new(""),
            password: TextEdit::new("").secure(),
            new_username: TextEdit::new(""),
            new_password: TextEdit::new("").secure(),
            password_shown: false,
            address_check: AddressCheck::Empty,
        }
    }
}

/// Which step shows, with only what is valid in it.
pub(crate) enum Screen {
    /// The address field and Connect.
    Address {
        phase: Phase,
        /// Whether Connect was pressed since the last edit, so the address's
        /// own problems show even while the field has focus.
        submitted: bool,
    },
    /// A saved server's account steps.
    Account {
        server: Server,
        form: Form,
        phase: Phase,
    },
}

/// Whether the step waits for the SDK, and the feedback line when it doesn't.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Editing(Option<Line>),
    /// A request runs; the fields keep their text and take no input.
    Busy,
}

/// The account step on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Form {
    /// The server's password, asked before the account.
    Unlock {
        /// Where unlocking leads.
        then: AccountForm,
        /// The access an earlier unlock gave has lapsed.
        lapsed: bool,
    },
    Account(AccountForm),
}

/// The two forms that name an account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AccountForm {
    SignIn,
    CreateAccount,
}

/// The server the account steps are for.
pub(crate) struct Server {
    pub(crate) id: ServerId,
    /// The name it gave, or its address while it has given none.
    pub(crate) name: String,
    /// Its address as shown under the name.
    pub(crate) address: String,
    pub(crate) icon: Option<Image>,
    /// Whether an unlock succeeded in this flow: if the server asks for its
    /// password again, the access expired.
    unlocked: bool,
    /// The saved account signing in again after its session expired; `None`
    /// for a new account.
    pub(crate) renewing: Option<Renewing>,
}

/// A saved account signing in again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Renewing {
    pub(crate) account: AccountId,
    /// Whether the keyring still holds its credentials: then its own sign-in
    /// handle checks the same person signs in; without them the server's
    /// handle signs in and the flow checks.
    pub(crate) credentials: Credentials,
}

impl SignIn {
    /// The address step, its field focused.
    pub(crate) fn new(host: Host, fields: &mut Fields) -> SignIn {
        fields.address.focused = true;
        SignIn {
            host,
            screen: Screen::Address {
                phase: Phase::Editing(None),
                submitted: false,
            },
            auth: None,
            abandoned: Arc::default(),
        }
    }

    /// Sign in again for `account`, whose session expired: its username filled
    /// in and the password focused. Also returns the request for the server's
    /// name and icon, which the screen shows when they arrive.
    pub(crate) fn renewing(
        host: Host,
        account: &SignedOut,
        fields: &mut Fields,
    ) -> (SignIn, Effect) {
        let flow = SignIn {
            host,
            screen: renewing_screen(account, fields),
            auth: None,
            abandoned: Arc::default(),
        };
        let discover = Effect::Discover {
            address: account.address.clone(),
        };
        (flow, discover)
    }

    pub(crate) fn host(&self) -> Host {
        self.host
    }

    /// The person closed the screens. Returns the server they leave behind
    /// when it may have no account: `requests` removes it unless one is
    /// saved. A probe still running removes the server it adds.
    pub(crate) fn abandon(&self) -> Option<ServerId> {
        self.abandoned.store(true, Ordering::Relaxed);
        match &self.screen {
            Screen::Account {
                server,
                phase: Phase::Editing(_),
                ..
            } if server.renewing.is_none() => Some(server.id),
            // A request in flight may yet save the account; the next start
            // finds the server already added if it did not.
            Screen::Account { .. } | Screen::Address { .. } => None,
        }
    }

    pub(crate) fn screen(&self) -> &Screen {
        &self.screen
    }

    /// Sets the icon the shell decoded for the server on screen.
    pub(crate) fn set_icon(&mut self, icon: Image) {
        if let Screen::Account { server, .. } = &mut self.screen {
            server.icon = Some(icon);
        }
    }

    /// The address step's line: an answer from the SDK, or the address's own
    /// problem once the field loses focus or Connect is pressed.
    pub(crate) fn address_line(&self, fields: &Fields) -> Option<Line> {
        let Screen::Address { phase, submitted } = &self.screen else {
            return None;
        };
        if let Phase::Editing(Some(line)) = phase {
            return Some(line.clone());
        }
        if !*submitted && fields.address.focused {
            return None;
        }
        match &fields.address_check {
            AddressCheck::Invalid(problem) => Some(Line::Address(*problem)),
            AddressCheck::Valid {
                unencrypted: true, ..
            } => Some(Line::Unencrypted),
            AddressCheck::Valid { .. } | AddressCheck::Empty => None,
        }
    }
}

/// Sign in for `account` again: its username filled in, the password
/// focused, its address standing in for the name until the server gives one.
fn renewing_screen(account: &SignedOut, fields: &mut Fields) -> Screen {
    fields.username.replace_text(account.username.clone());
    fields.password.focused = true;
    let address = address::shown(&account.address).to_owned();
    let server = Server {
        id: account.id.server(),
        name: address.clone(),
        address,
        icon: None,
        unlocked: false,
        renewing: Some(Renewing {
            account: account.id,
            credentials: account.credentials,
        }),
    };
    Screen::Account {
        server,
        form: Form::Account(AccountForm::SignIn),
        phase: Phase::Editing(None),
    }
}
