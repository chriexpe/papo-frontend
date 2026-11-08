//! Owns the Papo wire protocol and transports; state, storage and retry policy belong to `papo-sdk`.

mod error;
pub mod gateway;
mod history;
mod rest;
pub mod wire;

pub use error::{Error, ProblemKind};
pub use history::{HistoryCursor, HistoryOrder, HistoryQuery};
pub use rest::RestClient;
/// Backend UUIDs are opaque identities, never chronological cursors.
pub use uuid::Uuid;
