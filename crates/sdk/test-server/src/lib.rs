//! Owns a deterministic loopback test backend; leaves production clients and protocol coverage to `papo-api`.
//!
//! Supported routes are public `GET /server`, `POST /auth/register`, `POST /auth/login`,
//! `POST /auth/login_server`, `GET /auth/whoami`, `POST /auth/logout`, `POST /auth/refresh`,
//! `GET /auth/connected_devices`, `POST /auth/drop_connection`, `POST /auth/password_reset`,
//! and `GET /ws`. History supports `GET /channels/{id}/messages` and
//! `GET /channels/{id}/pinned`. Reconciliation supports `GET /channels`, channel permission snapshots,
//! `GET /roles`, paginated `GET /users`, individual profiles and profile/summary batches,
//! `GET /dms`, and `GET /users/blocks`. Read marks are account-scoped channel fields,
//! not a separate listing route. `PUT /channels/{id}/read` returns empty 204 and
//! advances only by the pinned `(created_at,id)` order, never by acknowledgement time.
//! [`TestServer::set_read_acknowledged_at`] controls HTTP acknowledgement time;
//! [`TestServer::advance_read_mark`] silently models another device. Existing REST scripts
//! and [`TestServer::hold_next_rest`] combine with [`TestServer::read_request_counts`]
//! for debounce, retry and inflight assertions. Profiles are fetched only when requested.
//! Reset links can be issued by the in-memory fixture capability only.
//! The gateway supports presence, typed channel/member/DM changes, heartbeat, invalid-event
//! errors and ordered scenario frames. Scripted controls keep authoritative state while
//! disconnected. Disable [`TestServer::set_gateway_accepting`] before disconnecting sockets
//! to keep reconnects rejected with HTTP 503 until mutations are complete.
//! Server-info, role-definition, overwrite-only, block and read-mark changes
//! have no live event in the pinned backend; this fixture does not invent one.
//! [`HistorySeed`] recipes in [`Config::history`] create content-free synthetic channels,
//! including equal-microsecond timestamps and explicit deterministic message UUID ranges.
//! [`TestServer::seed_messages`] silently seeds exact wire messages; mutation controls
//! update REST state and deliver pinned event shapes to readable accounts. Own reactions
//! are account-scoped; message payloads have no invented pin flag or deletion tombstone.
//! [`TestServer::set_channel_readable`] scripts an authorization result, not a role/DM
//! permission engine. Existing raw event controls deliberately bypass that filtering.
//! It is not a JWT implementation, expiring-session simulator or voice server.
//! Cookies are opaque deterministic capabilities and keep the backend's Secure attributes;
//! plain-HTTP test clients must explicitly forward them. Nothing is logged.

mod auth;
mod channel_list;
mod config;
mod controls;
mod encoded_media;
mod gateway;
mod history;
mod history_controls;
mod history_http;
mod history_query;
mod history_seed;
mod media;
pub use encoded_media::{EncodedMedia, EncodedMediaObservation};
mod member_requests;
mod message_requests;
pub use message_requests::OutgoingRequestCounts;
mod members;
mod presence;
mod read_marks;
mod read_requests;
mod request;
mod resources;
mod response;
mod rest_hold;
mod server;

pub use config::{Config, Failure, GatewayStep, RestStep};
pub use history_seed::HistorySeed;
pub use media::{MediaRequest, MediaTestServer};
pub use member_requests::MemberRequestCounts;
pub use read_requests::ReadRequestCounts;
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
