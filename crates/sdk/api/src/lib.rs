//! Owns the Papo wire protocol and transports; state, storage and retry policy belong to `papo-sdk`.

pub mod administration;
mod auth;
mod credentials;
pub use credentials::CredentialSnapshot;
mod embeds;
mod error;
pub mod gateway;
mod health;
mod history;
pub mod media;
mod message_upload;
mod pagination;
mod people;
mod rest;
mod social;
mod voice;
pub mod wire;

pub use error::{Error, ProblemKind};
pub use history::{HistoryCursor, HistoryOrder, HistoryQuery};
pub use message_upload::{
    AttachmentByteStream, MessageAttachment, MessageUpload, StreamedMessageAttachment,
    StreamedMessageUpload,
};
pub use pagination::Cursor;
pub use rest::RestClient;
/// Backend UUIDs are opaque identities, never chronological cursors.
pub use uuid::Uuid;
pub(crate) use wire::routes;
