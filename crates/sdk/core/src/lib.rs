//! Client, account sessions, authentication, reconnect policy and durable device state.
//! Wire transport belongs to `papo-api`; UI projections and platform integration live above this crate.

mod activity;
pub use activity::{ActivitySnapshot, Presence, PresenceStatus, Typing};
mod account;
mod account_cache;
mod auth;
mod client;
mod client_error;
mod client_handle;
mod connection;
mod connection_driver;
mod disk_budget;
mod generation;
pub use disk_budget::{DiskBudget, DiskError, DiskFault, DiskReport, DiskStatus};
mod expressions;
pub use expressions::{
    CatalogPage, ExpressionError, ExpressionService, ExpressionSnapshot, Gif, GiphyConfig, Sticker,
};
pub use expressions::{ExpressionFault, ExpressionUpdate};
mod media;
pub use media::{
    EmojiMetadata, EmojiPage, MediaData, MediaError, MediaRange, MediaService, MediaSource,
};
mod history;
pub use history::{
    HistoryBudget, HistoryChunk, HistoryClosedReason, HistoryDiagnostics, HistoryError, HistoryGap,
    HistoryHandle, HistoryPage, HistorySnapshot, HistorySource, HistorySubscription, HistoryUpdate,
    HistoryWork,
};
pub use papo_api::wire::Message;
pub use papo_api::{HistoryCursor, HistoryOrder, HistoryQuery, Uuid};
mod observable;
mod outgoing;
pub use outgoing::OutgoingFault;
pub use outgoing::{
    Draft, DraftBody, MessageTarget, Operation, OperationFailure, OperationKind, OperationState,
    OutgoingAttachment, OutgoingError, OutgoingMessage, OutgoingService, OutgoingSnapshot,
    RetryDecision,
};
pub use papo_api::wire::{EmbedInput, EmbedMediaInput};
pub mod permissions;
mod private_fs;
mod profiles;
mod read_points;
mod reading;
pub use reading::{ReadPoint, ReadState, ReadingError, ReadingSnapshot, UnreadCount};
mod reconcile;
mod reconcile_event;
mod reconcile_fetch;
mod resources;
pub use account_cache::{AccountCache, AccountCaches, CacheError, CacheResource};
pub use papo_api::wire::{
    Channel, DirectConversation, ExternalAuthor, LoginRequest, LoginServerRequest, RegisterRequest,
    Role, RoleSummary, UserProfile, UserSummary,
};
pub use read_points::ServerReadPoint;
pub use resources::ResourceSnapshot;
mod session;
mod session_pool;

pub use auth::{AuthService, AuthState};
pub use client::{Client, ClientConfig};
pub use client_error::ClientError;
pub use client_handle::{ClientHandle, RegistrySnapshot};
pub use connection::{Backoff, ConnectionState, ConnectionStatus, Jitter, RuntimeJitter};
pub use generation::{Generated, Generation};
pub use session::{Session, SessionEventError, SessionEvents};
mod data_root;
mod registry;
mod registry_record;
pub mod removal;
pub mod secret;
mod server;
mod state_schema;
mod storage;
mod storage_error;

pub use storage::{Storage, StorageConfig, StorageShutdown};
pub mod storage_worker;

#[cfg(test)]
mod client_lifecycle_tests;
#[cfg(all(test, target_os = "linux"))]
mod crash_tests;
#[cfg(test)]
mod registry_tests;
#[cfg(test)]
mod removal_credential_tests;
#[cfg(test)]
mod removal_state_tests;
#[cfg(test)]
mod removal_tests;
#[cfg(test)]
mod storage_lifecycle_tests;
#[cfg(test)]
mod storage_tests;

pub use account::AccountId;
pub use data_root::DataRoot;
pub use registry::Registry;
pub use registry_record::{SavedAccount, SavedServer, UnixMicros};
pub use server::{ServerAddress, ServerAddressError, ServerId};
pub use storage_error::StorageError;

#[cfg(feature = "test-support")]
pub use outgoing::{OutgoingDispatchPhase, OutgoingDispatchProbe};
