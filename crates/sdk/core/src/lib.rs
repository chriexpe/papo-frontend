//! Device identity and durable storage for the Papo SDK; wire transport belongs to `papo-api`.
//! Session policy, view projections and platform integration live above these boundaries.

mod account;
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
