//! Proves SDK session policy over real loopback sockets; pure retry clocks stay in core unit tests.

#![cfg(feature = "test-support")]

#[path = "loopback/auth.rs"]
mod auth;
mod fixture;
#[path = "loopback/lifecycle.rs"]
mod lifecycle;
#[path = "loopback/removal.rs"]
mod removal;
