//! What the flow hears ([`Event`]: the person's presses and the SDK's
//! answers, already sorted) and what it asks for ([`Effect`]).
//!
//! Owns only the shapes; `transitions` decides, `requests` carries out.

use papo_sdk::ServerId;

use super::{AccountForm, AlreadyAdded};

/// What happened, from the person or the SDK.
pub(crate) enum Event {
    /// A field's text changed.
    Edited,
    /// Return, the primary button, or a line's Try again.
    Submit,
    /// The already-added line's Open.
    Open,
    ChangeServer,
    ShowForm(AccountForm),
    TogglePassword,
    Probed(Probe),
    /// The renewing server's discovery, fetched for its name and icon.
    Discovered(Option<Discovery>),
    Unlocked(Result<(), Failure>),
    SignedIn(Result<(), Failure>),
    Created(Created),
}

/// The address step's answer.
pub(crate) enum Probe {
    /// The server is saved now.
    Added {
        id: ServerId,
        discovery: Discovery,
    },
    /// It was saved before; nothing changed.
    AlreadyAdded(AlreadyAdded),
    Failed(ProbeFailure),
}

/// What a server said about itself, as the screens use it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct Discovery {
    pub(crate) name: Option<String>,
    pub(crate) public: bool,
    /// Encoded image bytes, not decoded yet.
    pub(crate) icon: Option<Vec<u8>>,
}

/// Why the probe found no server, as `docs/design.md` §7 words it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProbeFailure {
    Unreachable,
    NotSetUp,
    NotPapo,
}

/// Why an account step's request failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Failure {
    /// The server wants its password first.
    AccessRequired,
    WrongCredentials,
    Banned,
    RateLimited,
    NoResponse,
    NameTaken,
    Keyring,
    Other,
}

/// Create account's answer: it registers, then signs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Created {
    SignedIn,
    /// Registering failed; no account exists.
    NotRegistered(Failure),
    /// The account exists but signing in failed.
    NotSignedIn(Failure),
}

/// What [`SignIn::step`](super::SignIn::step) asks the shell to do.
#[derive(PartialEq, Eq)]
pub(crate) enum Effect {
    Probe {
        address: String,
    },
    /// Fetch the renewing server's name and icon.
    Discover {
        address: String,
    },
    Unlock {
        password: String,
    },
    SignIn {
        username: String,
        password: String,
    },
    CreateAccount {
        username: String,
        password: String,
    },
    /// Change server leaves no server without an account behind.
    RemoveServer(ServerId),
    /// Decode the server's icon for the account steps.
    LoadIcon(Vec<u8>),
    /// Signed in to server `id`, named `name`: close the screen, show the
    /// server and say so.
    Finish {
        id: ServerId,
        name: String,
    },
    /// Show the app on a server that has an account already.
    OpenServer(ServerId),
}

impl std::fmt::Debug for Effect {
    /// Names the effect only: addresses, names and passwords stay out of logs
    /// and test output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Effect::Probe { .. } => "Probe",
            Effect::Discover { .. } => "Discover",
            Effect::Unlock { .. } => "Unlock",
            Effect::SignIn { .. } => "SignIn",
            Effect::CreateAccount { .. } => "CreateAccount",
            Effect::RemoveServer(_) => "RemoveServer",
            Effect::LoadIcon(_) => "LoadIcon",
            Effect::Finish { .. } => "Finish",
            Effect::OpenServer(_) => "OpenServer",
        };
        f.write_str(name)
    }
}

impl std::fmt::Debug for Discovery {
    /// Leaves the server's own text out of logs and test output.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Discovery")
            .field("named", &self.name.is_some())
            .field("public", &self.public)
            .field("icon", &self.icon.is_some())
            .finish()
    }
}
