//! What a server says about itself before any account exists (`GET /server`): the
//! non-saving probe `ClientHandle::discover`, the discovery `add_server` returns, and why a
//! probe failed. Nothing here is persisted; the registry keeps only the address. Display and
//! wording belong to clients.

use base64::Engine;

use crate::{ClientError, ClientHandle, ServerAddress, ServerId};

/// The largest decoded server icon kept from discovery, in bytes. A larger icon is dropped.
pub const MAX_ICON_BYTES: usize = 256 * 1024;

/// The longest base64 text that can decode to [`MAX_ICON_BYTES`], checked before decoding.
const MAX_ICON_BASE64_CHARS: usize = MAX_ICON_BYTES.div_ceil(3) * 4;

/// A server's public self-description, read without signing in. Held only in memory.
/// `Debug` omits the server's own text (its name and icon format).
#[derive(Clone, PartialEq, Eq)]
pub struct ServerDiscovery {
    /// The server's display name, as the server sent it; `None` when it sent none.
    pub name: Option<String>,
    /// False when the server asks for its shared password before registration or sign-in.
    /// A server that omits the flag counts as public; if it does need a password, sign-in
    /// still reports `AuthState::NeedsServerPassword`.
    pub public: bool,
    /// The server's icon; `None` when it has none or sent one that is malformed or larger
    /// than [`MAX_ICON_BYTES`]. The bytes are not decoded as an image here.
    pub icon: Option<ServerIcon>,
}

/// A server icon's encoded image bytes, at most [`MAX_ICON_BYTES`].
#[derive(Clone, PartialEq, Eq)]
pub struct ServerIcon {
    /// The image format the server declared (for example `png`), trimmed; `None` when blank.
    pub format: Option<String>,
    /// The encoded image.
    pub bytes: Vec<u8>,
}

impl std::fmt::Debug for ServerDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerDiscovery")
            .field("named", &self.name.is_some())
            .field("public", &self.public)
            .field("icon", &self.icon)
            .finish()
    }
}

impl std::fmt::Debug for ServerIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerIcon")
            .field("bytes", &self.bytes.len())
            .finish_non_exhaustive()
    }
}

/// A server just saved by `add_server`, with the discovery its probe fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AddedServer {
    /// The saved server's identity.
    pub id: ServerId,
    /// What the server said about itself while it was probed.
    pub discovery: ServerDiscovery,
}

/// Why `GET /server` gave no discovery, in the categories clients word differently.
///
/// Redirects are not followed, so a 3xx (for example `http` to `https`) is `NotPapo`; so is
/// a connection that drops while the reply body is read, which the transport reports as an
/// undecodable reply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DiscoveryFailure {
    /// A Papo server answered that it has not been set up yet: 404 with the `not-found`
    /// problem.
    NotSetUp,
    /// Something answered that is not a Papo server: an undecodable reply, an HTTP error
    /// without a problem document, or a problem type Papo does not define.
    NotPapo,
    /// No usable answer: the connection failed or timed out, the server was rate limited or
    /// failed (5xx), or a Papo server refused discovery with another of its problems.
    /// Trying again may help.
    Unreachable,
}

impl DiscoveryFailure {
    /// Classifies an `add_server` or `discover` failure. `None` means the probe was not what
    /// failed: an invalid address, a server already saved, storage, or a closed client.
    #[must_use]
    pub fn classify(error: &ClientError) -> Option<Self> {
        use papo_api::Error as Api;
        let api = match error {
            ClientError::Timeout => return Some(Self::Unreachable),
            ClientError::Api(api) => api,
            _ => return None,
        };
        Some(match api {
            Api::Transport | Api::NotDispatched => Self::Unreachable,
            Api::Http { status } | Api::Problem { status, .. }
                if *status == 429 || (500..=599).contains(status) =>
            {
                Self::Unreachable
            }
            Api::Problem {
                status: 404,
                problem,
            } if problem.kind() == papo_api::ProblemKind::NotFound => Self::NotSetUp,
            Api::Problem { problem, .. } if problem.kind() != papo_api::ProblemKind::Unknown => {
                // A problem type Papo defines proves a Papo server answered.
                Self::Unreachable
            }
            Api::Decode | Api::Http { .. } | Api::Problem { .. } => Self::NotPapo,
            // A rejected address or credential snapshot never reached the server.
            _ => return None,
        })
    }
}

impl ClientHandle {
    /// Validates and probes `/server` without saving anything, for a server that is already
    /// saved or not yet wanted.
    /// # Errors
    /// Returns validation, probe or closed-client errors; [`DiscoveryFailure::classify`]
    /// sorts probe failures.
    pub async fn discover(&self, address: &str) -> Result<ServerDiscovery, ClientError> {
        let address = ServerAddress::parse(address)?;
        let this = self.clone();
        self.dispatch(async move {
            let rest = this.inner.transport(&address, None)?;
            let server = this.inner.request(rest.server()).await?;
            Ok(ServerDiscovery::from_wire(server))
        })
        .await
    }
}

impl ServerDiscovery {
    /// Keeps what clients show from the wire reply; the icon is decoded and capped here.
    pub(crate) fn from_wire(server: papo_api::wire::Server) -> Self {
        Self {
            name: server.name,
            public: server.public.unwrap_or(true),
            icon: icon(server.icon_blob, server.icon_format),
        }
    }
}

/// The icon, strictly standard base64 as the backend encodes bytes; anything else is dropped.
fn icon(blob: Option<String>, format: Option<String>) -> Option<ServerIcon> {
    let blob = blob.filter(|blob| !blob.is_empty())?;
    if blob.len() > MAX_ICON_BASE64_CHARS {
        return None;
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(blob)
        .ok()
        .filter(|bytes| !bytes.is_empty() && bytes.len() <= MAX_ICON_BYTES)?;
    let format = format
        .map(|format| format.trim().to_owned())
        .filter(|format| !format.is_empty());
    Some(ServerIcon { format, bytes })
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
