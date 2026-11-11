//! Client, account sessions, authentication, reconnect policy and durable device state.
//! Wire transport belongs to `papo-api`; UI projections and platform integration live above this crate.

mod account;
mod account_cache;
mod auth;
mod client;
mod client_error;
mod client_handle;
mod connection;
mod connection_driver;
mod generation;
mod history;
pub use history::{
    HistoryChunk, HistoryClosedReason, HistoryError, HistoryGap, HistoryHandle, HistoryPage,
    HistorySnapshot, HistorySource, HistorySubscription, HistoryUpdate,
};
pub use papo_api::wire::Message;
pub use papo_api::{HistoryCursor, HistoryOrder, HistoryQuery, Uuid};
mod observable;
mod profiles;
mod read_points;
mod reading;
pub use reading::{ReadPoint, ReadState, ReadingError, ReadingSnapshot, UnreadCount};
mod reconcile;
mod reconcile_event;
mod reconcile_fetch;
mod resources;
pub use account_cache::{AccountCache, AccountCaches, CacheError, CacheResource};
pub use papo_api::wire::{Channel, LoginRequest, LoginServerRequest, RegisterRequest};
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
