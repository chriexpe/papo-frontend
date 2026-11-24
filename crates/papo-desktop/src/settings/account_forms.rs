//! What the account's pages choose and type (`docs/design.md` §8): the
//! persisted presence, and Change password's form with what its Save found
//! wrong.
//!
//! Owns these values; the rows are `rows::account`', the calls the shell's
//! (`shell/settings_account`).

use rune::TextEdit;

/// A persisted presence the account chooses (§8, Profile › Presence).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Presence {
    /// Automatic: the backend's own, from the connection.
    Available,
    Away,
    Busy,
}

/// Change password's form (§8: new and repeat, with §7's hint).
pub(crate) struct PasswordForm {
    pub(crate) new: TextEdit,
    pub(crate) repeat: TextEdit,
    /// The fields show their characters.
    pub(crate) shown: bool,
    /// This device signs in again with the new password (on at first):
    /// the backend signs every session out.
    pub(crate) stay: bool,
    /// What the last Save found wrong, if anything.
    pub(crate) problem: Option<PasswordProblem>,
}

/// Why Change password's Save did not send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PasswordProblem {
    /// The new password misses §7's rules.
    Weak,
    /// The two fields differ.
    Mismatch,
}

impl Default for PasswordForm {
    fn default() -> PasswordForm {
        PasswordForm {
            new: TextEdit::new(""),
            repeat: TextEdit::new(""),
            shown: false,
            stay: true,
            problem: None,
        }
    }
}
