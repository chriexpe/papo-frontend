//! This desktop client's own non-secret profile, independent of every other client.
//!
//! The registry remembers server addresses, order and selection. Core supplies
//! address validation and stable server keys; it still owns account and message
//! state. Credentials never enter this format. Filesystem work belongs on a
//! worker, not GPUI's foreground. No import, cache or auth operation runs here.

mod atomic;
mod model;
mod profile;
mod service;

#[cfg(test)]
mod tests;

pub use model::{Change, SavedServer, ServerList};
pub use profile::Profile;

pub(crate) use service::{install, stop_before_session_join};

/// The versioned desktop registry, unrelated to eframe's application state.
const REGISTRY_NAME: &str = "servers.json";

/// A sanitized persistence failure. Neither paths nor file contents are retained.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum PersistenceError {
    /// The operating system has no application data directory.
    #[error("application data directory is unavailable")]
    NoDataDirectory,
    /// The profile root or a registry file is not an ordinary directory/file.
    #[error("profile contains an unsupported filesystem entry")]
    InvalidPath,
    /// The operating system rejected profile access.
    #[error("profile filesystem operation failed ({0})")]
    Io(std::io::ErrorKind),
    /// Another cooperating desktop process owns this profile.
    #[error("profile is already in use")]
    Locked,
    /// A server address is not an accepted non-secret HTTP(S) base address.
    #[error("invalid server address")]
    InvalidAddress,
    /// The address has the same core identity as an existing entry.
    #[error("server is already saved")]
    DuplicateServer,
    /// The requested server is not in the saved list.
    #[error("server is not saved")]
    UnknownServer,
    /// A new order is not an exact permutation of the saved server keys.
    #[error("invalid server order")]
    InvalidOrder,
    /// The document is malformed or violates the registry invariants.
    #[error("invalid server registry document")]
    InvalidDocument,
    /// This client cannot safely read or rewrite the document's version.
    #[error("unsupported server registry version")]
    UnsupportedVersion,
    /// A defensive address, server-count or document-size limit was exceeded.
    #[error("server registry exceeds its size limit")]
    TooLarge,
    /// Replacement succeeded, but syncing its directory failed. The new state
    /// is visible; power-loss durability is not confirmed. Read the snapshot.
    #[error("server registry was replaced but directory durability is unconfirmed")]
    DurabilityUnknown,
    /// The bounded queue already holds a request; nothing was accepted.
    #[error("persistence service is busy")]
    Busy,
    /// The application stopped accepting persistence work.
    #[error("persistence service is stopping")]
    Stopped,
    /// The operating system could not start the persistence worker.
    #[error("could not start the persistence worker")]
    Worker,
}

impl From<std::io::Error> for PersistenceError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.kind())
    }
}
