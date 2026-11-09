//! Owns a deterministic loopback test backend; leaves production clients and protocol coverage to `papo-api`.
//!
//! Supported routes are public `GET /server`, `POST /auth/register`, `POST /auth/login`,
//! `POST /auth/login_server`, `GET /auth/whoami`, `POST /auth/logout`, `POST /auth/refresh`,
//! `GET /auth/connected_devices`, `POST /auth/drop_connection`, `POST /auth/password_reset`,
//! and `GET /ws`. Reconciliation supports `GET /channels`, channel permission snapshots,
//! `GET /roles`, paginated `GET /users`, individual profiles and profile/summary batches,
//! `GET /dms`, and `GET /users/blocks`. Read marks are account-scoped channel fields,
//! not a separate listing route. Profiles are fetched only when requested.
//! Reset links can be issued by the in-memory fixture capability only.
//! The gateway supports presence, typed channel/member/DM changes, heartbeat, invalid-event
//! errors and ordered scenario frames. Scripted controls keep authoritative state while
//! disconnected. Disable [`TestServer::set_gateway_accepting`] before disconnecting sockets
//! to keep reconnects rejected with HTTP 503 until mutations are complete.
//! Server-info, role-definition, overwrite-only, block and read-mark changes
//! have no live event in the pinned backend; this fixture does not invent one.
//! It is not a JWT implementation, expiring-session simulator or voice server.
//! Cookies are opaque deterministic capabilities and keep the backend's Secure attributes;
//! plain-HTTP test clients must explicitly forward them. Nothing is logged.

mod auth;
mod config;
mod controls;
mod gateway;
mod media;
mod members;
mod presence;
mod request;
mod resources;
mod response;
mod rest_hold;
mod server;

pub use config::{Config, Failure, GatewayStep, RestStep};
pub use media::{MediaRequest, MediaTestServer};
pub use rest_hold::RestHold;
pub use server::TestServer;

/// Sanitized failures; no input, credential, address or event payload is retained.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The scenario has an invalid limit, route override or HTTP failure status.
    #[error("invalid test-server configuration")]
    Configuration,
    /// A runtime-supplied configuration could not be read or decoded.
    #[error("test-server configuration could not be loaded")]
    ConfigurationRead,
    /// The requested loopback listener could not be bound.
    #[error("loopback listener could not be bound")]
    Bind,
    /// A listener operation failed.
    #[error("loopback listener operation failed")]
    Listener,
    /// A server task failed unexpectedly.
    #[error("test-server task failed")]
    Task,
    /// HTTP or gateway transport failed.
    #[error("test-server connection failed")]
    Connection,
    /// A command-line option was missing or invalid.
    #[error("invalid test-server command-line options")]
    Arguments,
    /// The Tokio runtime could not be created.
    #[error("test-server runtime could not be created")]
    Runtime,
    /// The shutdown signal could not be received.
    #[error("test-server shutdown signal failed")]
    Signal,
}
