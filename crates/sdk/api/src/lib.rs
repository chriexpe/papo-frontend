//! Owns the Papo wire protocol and transports; state, storage and retry policy belong to `papo-sdk`.

pub mod administration;
mod error;
pub mod gateway;
mod history;
pub mod media;
pub mod people;
pub mod protocol_routes;
mod rest;
pub mod social;
pub mod wire;

pub use error::{Error, ProblemKind};
pub use history::{HistoryCursor, HistoryOrder, HistoryQuery};
pub use rest::{MessageAttachment, MessageUpload, RestClient};
/// Backend UUIDs are opaque identities, never chronological cursors.
pub use uuid::Uuid;
