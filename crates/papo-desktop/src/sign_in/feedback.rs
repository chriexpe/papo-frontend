//! The feedback line of the server and account screens: what it says, its
//! tone and its one action (`docs/design.md` §7, Messages).
//!
//! Owns which line each failure gets and how it is coloured and announced;
//! the words are `strings`', where the line shows is the view's.

use papo_sdk::ServerId;

use super::{AddressProblem, Failure};

/// The feedback line: under the address field on the server step, above the
/// button on the account steps (`docs/design.md` §7, Messages).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Line {
    Address(AddressProblem),
    Unencrypted,
    Unreachable,
    NotSetUp,
    NotPapo,
    /// The server is already saved, under `name`.
    AlreadyAdded(Box<AlreadyAdded>),
    WrongServerPassword,
    WrongCredentials,
    Banned,
    RateLimited,
    NoResponse,
    NameTaken,
    /// The keyring refused to keep the credentials.
    Keyring,
    /// A failure none of the others names.
    Failed,
}

/// A server the address step found already saved.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct AlreadyAdded {
    pub(crate) id: ServerId,
    /// Its name, or its address when it did not answer.
    pub(crate) name: String,
    pub(crate) address: String,
    /// Whether an account is saved for it: Open then shows the app instead of
    /// the account steps.
    pub(crate) has_account: bool,
    pub(crate) public: bool,
    pub(crate) icon: Option<Vec<u8>>,
}

/// How a line is coloured, and how assistive technology announces it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tone {
    Danger,
    Warning,
}

/// The one action a line can carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LineAction {
    TryAgain,
    Open,
}

impl Line {
    pub(crate) fn tone(&self) -> Tone {
        match self {
            Line::Unencrypted | Line::AlreadyAdded(_) | Line::RateLimited => Tone::Warning,
            _ => Tone::Danger,
        }
    }

    pub(crate) fn action(&self) -> Option<LineAction> {
        match self {
            Line::Unreachable | Line::NoResponse => Some(LineAction::TryAgain),
            Line::AlreadyAdded(_) => Some(LineAction::Open),
            _ => None,
        }
    }
}

/// The line for a failure every account step words the same way.
pub(super) fn line_for(failure: Failure) -> Line {
    match failure {
        Failure::WrongCredentials => Line::WrongCredentials,
        Failure::Banned => Line::Banned,
        Failure::RateLimited => Line::RateLimited,
        Failure::NoResponse => Line::NoResponse,
        Failure::NameTaken => Line::NameTaken,
        Failure::Keyring => Line::Keyring,
        Failure::AccessRequired | Failure::Other => Line::Failed,
    }
}

impl std::fmt::Debug for AlreadyAdded {
    /// Leaves the server's name and address out of logs and test output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AlreadyAdded")
            .field("id", &self.id)
            .field("has_account", &self.has_account)
            .field("public", &self.public)
            .finish_non_exhaustive()
    }
}
