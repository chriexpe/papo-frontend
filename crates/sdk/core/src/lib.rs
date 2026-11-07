//! Device and account identity for the Papo SDK; wire contracts belong to `papo-api`.
//! View projections and platform integration live above this crate.

mod account;
mod server;

pub use account::AccountId;
pub use server::{ServerAddress, ServerAddressError, ServerId};
