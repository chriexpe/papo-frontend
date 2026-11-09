//! Proves SDK session policy over real loopback sockets; pure retry clocks stay in core unit tests.

#![cfg(feature = "test-support")]

#[path = "loopback/auth.rs"]
mod auth;
#[path = "loopback/cache.rs"]
mod cache;
mod fixture;
#[path = "loopback/lifecycle.rs"]
mod lifecycle;
#[path = "loopback/member_refresh.rs"]
mod member_refresh;
#[path = "loopback/profiles.rs"]
mod profiles;
#[path = "loopback/reconcile.rs"]
mod reconcile;
#[path = "loopback/removal.rs"]
mod removal;
