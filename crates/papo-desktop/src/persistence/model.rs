//! Validated, non-secret server registry values and their versioned JSON format.
//!
//! Core owns address policy and server identity. This module preserves order and
//! explicit selection, rejecting a whole change or document when an invariant
//! fails. It performs no I/O, UI, authentication, credential or cache operations;
//! the repository owns the file-size limit and atomic replacement.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Deserializer, Serialize};

use papo_core::{recovery::server_address, server_key};

use super::PersistenceError;

/// Local defensive policy: at most 1,024 saved servers, independent of backend
/// limits. This bounds registry validation and replacement work on untrusted data.
const MAX_SERVERS: usize = 1024;

/// Local defensive policy: at most 8,192 UTF-8 bytes per raw and canonical base
/// address. Checking both bounds also limits expansion during URL normalization.
const MAX_ADDRESS_BYTES: usize = 8192;

/// The only registry schema this desktop client can read and write.
const DOCUMENT_VERSION: u64 = 1;

/// A core-validated base address and its derived stable core identity.
///
/// Fields are private so an address cannot diverge from its key. No `Debug`
/// implementation is provided: addresses must not leak through diagnostic output.
#[derive(Clone, PartialEq, Eq)]
pub struct SavedServer {
    url: String,
    key: String,
}

impl SavedServer {
    /// Returns the canonical, non-secret HTTP(S) base address from core policy.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Returns the stable identity produced by [`papo_core::server_key`].
    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Checks local byte bounds around core normalization, then derives identity.
    fn parse(raw: &str) -> Result<Self, PersistenceError> {
        check_address_size(raw)?;
        let url = server_address(raw).map_err(|_| PersistenceError::InvalidAddress)?;
        check_address_size(&url)?;
        let key = server_key(&url);
        Ok(Self { url, key })
    }
}

/// An ordered registry with unique core identities and an optional saved key.
///
/// Empty is valid. Adding never selects implicitly; removing the selected server
/// clears selection without choosing another. This value has no diagnostic
/// formatter because its addresses are private data.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ServerList {
    servers: Vec<SavedServer>,
    selected_server: Option<String>,
}

/// One explicit edit to the non-secret registry, with no authentication, cache or
/// credential side effects. Server references are saved core keys, not raw URLs.
///
/// No `Debug` implementation is provided because additions contain addresses.
pub enum Change {
    /// Append an address after core validation; do not change selection.
    Add {
        /// A raw HTTP(S) base address, canonicalized by core policy.
        url: String,
    },
    /// Remove a saved key, clearing selection only if that key was selected.
    Remove {
        /// The exact core key of an existing entry.
        server: String,
    },
    /// Select an existing key, or explicitly clear selection with `None`.
    Select {
        /// An existing entry's exact core key, or no selection.
        server: Option<String>,
    },
    /// Replace order with an exact permutation of every saved key.
    Reorder {
        /// All existing core keys, each exactly once, in the requested order.
        servers: Vec<String>,
    },
}

impl ServerList {
    /// Returns saved servers in their persisted order.
    #[must_use]
    pub fn servers(&self) -> &[SavedServer] {
        &self.servers
    }

    /// Returns the selected saved core key, or `None` for no selection.
    #[must_use]
    pub fn selected_server(&self) -> Option<&str> {
        self.selected_server.as_deref()
    }

    /// Creates a validated replacement without modifying this snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`PersistenceError::InvalidAddress`] for an unacceptable address,
    /// [`PersistenceError::DuplicateServer`] for an existing core identity,
    /// [`PersistenceError::UnknownServer`] for an absent removal or selection,
    /// and [`PersistenceError::InvalidOrder`] for a non-permutation. Returns
    /// [`PersistenceError::TooLarge`] when local address or server-count bounds
    /// are exceeded. An inconsistent internal snapshot returns
    /// [`PersistenceError::InvalidDocument`]. No partial replacement is returned.
    pub fn apply(&self, change: Change) -> Result<Self, PersistenceError> {
        let mut replacement = self.clone();
        match change {
            Change::Add { url } => {
                if replacement.servers.len() >= MAX_SERVERS {
                    return Err(PersistenceError::TooLarge);
                }
                let server = SavedServer::parse(&url)?;
                if replacement
                    .servers
                    .iter()
                    .any(|saved| saved.key == server.key)
                {
                    return Err(PersistenceError::DuplicateServer);
                }
                replacement.servers.push(server);
            }
            Change::Remove { server } => {
                let index = replacement
                    .servers
                    .iter()
                    .position(|saved| saved.key == server)
                    .ok_or(PersistenceError::UnknownServer)?;
                replacement.servers.remove(index);
                if replacement.selected_server.as_deref() == Some(server.as_str()) {
                    replacement.selected_server = None;
                }
            }
            Change::Select { server } => {
                if server
                    .as_ref()
                    .is_some_and(|key| !replacement.servers.iter().any(|saved| saved.key == *key))
                {
                    return Err(PersistenceError::UnknownServer);
                }
                replacement.selected_server = server;
            }
            Change::Reorder { servers } => {
                replacement.servers = self.reordered(&servers)?;
            }
        }
        replacement.validate()?;
        Ok(replacement)
    }

    fn reordered(&self, keys: &[String]) -> Result<Vec<SavedServer>, PersistenceError> {
        check_server_count(keys.len())?;
        if keys.len() != self.servers.len() {
            return Err(PersistenceError::InvalidOrder);
        }
        let mut remaining: HashMap<_, _> = self
            .servers
            .iter()
            .map(|saved| (saved.key.as_str(), saved))
            .collect();
        keys.iter()
            .map(|key| {
                remaining
                    .remove(key.as_str())
                    .cloned()
                    .ok_or(PersistenceError::InvalidOrder)
            })
            .collect()
    }

    /// Rejects inconsistent state rather than repairing entries or selection.
    fn validate(&self) -> Result<(), PersistenceError> {
        check_server_count(self.servers.len())?;
        let mut keys = HashSet::with_capacity(self.servers.len());
        for server in &self.servers {
            let canonical = SavedServer::parse(&server.url).map_err(document_error)?;
            if canonical != *server || !keys.insert(server.key.as_str()) {
                return Err(PersistenceError::InvalidDocument);
            }
        }
        if self
            .selected_server()
            .is_some_and(|selected| !keys.contains(selected))
        {
            return Err(PersistenceError::InvalidDocument);
        }
        Ok(())
    }
}

/// A permissive envelope checks version before interpreting any version-one
/// fields. Duplicate version fields remain invalid through derived deserialization.
#[derive(Deserialize)]
struct DocumentVersion {
    version: u64,
}

/// The complete version-one schema. No account, credential or cache fields exist.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u64,
    servers: Vec<String>,
    #[serde(deserialize_with = "required_selection")]
    selected_server: Option<String>,
}

/// Unlike serde's default handling of `Option`, a missing field is not `null`.
fn required_selection<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer)
}

/// Encodes exactly the non-secret version-one schema after checking invariants.
/// The repository must bound the resulting file before writing it.
///
/// # Errors
///
/// Returns [`PersistenceError::TooLarge`] for local model bounds, or
/// [`PersistenceError::InvalidDocument`] for an inconsistent snapshot or a
/// serialization failure. Raw serializer errors never escape with input data.
pub(super) fn encode(list: &ServerList) -> Result<Vec<u8>, PersistenceError> {
    list.validate()?;
    let document = Document {
        version: DOCUMENT_VERSION,
        servers: list.servers.iter().map(|saved| saved.url.clone()).collect(),
        selected_server: list.selected_server.clone(),
    };
    serde_json::to_vec(&document).map_err(|_| PersistenceError::InvalidDocument)
}

/// Loads an entire document without dropping duplicates or repairing selection.
/// Core normalization is allowed; server order and membership remain intact.
/// The repository must apply its file-size bound before passing bytes here.
///
/// # Errors
///
/// Returns [`PersistenceError::UnsupportedVersion`] for an unknown integer
/// version, [`PersistenceError::TooLarge`] for local model bounds, or
/// [`PersistenceError::InvalidDocument`] for malformed JSON, an invalid schema,
/// rejected addresses, duplicate core identities or an absent selected key.
pub(super) fn decode(bytes: &[u8]) -> Result<ServerList, PersistenceError> {
    let envelope: DocumentVersion =
        serde_json::from_slice(bytes).map_err(|_| PersistenceError::InvalidDocument)?;
    if envelope.version != DOCUMENT_VERSION {
        return Err(PersistenceError::UnsupportedVersion);
    }
    let document: Document =
        serde_json::from_slice(bytes).map_err(|_| PersistenceError::InvalidDocument)?;
    check_server_count(document.servers.len())?;
    let servers = document
        .servers
        .iter()
        .map(|raw| SavedServer::parse(raw).map_err(document_error))
        .collect::<Result<_, _>>()?;
    let list = ServerList {
        servers,
        selected_server: document.selected_server,
    };
    list.validate()?;
    Ok(list)
}

fn check_address_size(address: &str) -> Result<(), PersistenceError> {
    if address.len() > MAX_ADDRESS_BYTES {
        return Err(PersistenceError::TooLarge);
    }
    Ok(())
}

fn check_server_count(count: usize) -> Result<(), PersistenceError> {
    if count > MAX_SERVERS {
        return Err(PersistenceError::TooLarge);
    }
    Ok(())
}

/// Loaded content uses a single invalid-document category, except for bounds.
fn document_error(error: PersistenceError) -> PersistenceError {
    match error {
        PersistenceError::TooLarge => PersistenceError::TooLarge,
        _ => PersistenceError::InvalidDocument,
    }
}

#[cfg(test)]
#[path = "model_tests.rs"]
mod tests;
