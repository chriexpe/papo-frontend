//! Platform binding translations belong here; all policy and services belong to `papo-sdk-ui`.
//! Native widgets and navigation remain in the consuming application.

uniffi::setup_scaffolding!();

/// Opt-in integration proof only; not a production timeline service.
#[cfg(feature = "integration-proof")]
pub mod timeline_proof;
