//! Platform binding translations belong here; all policy and services belong to `papo-sdk-ui`.
//! Native widgets and navigation remain in the consuming application.

uniffi::setup_scaffolding!();

/// Android's native callback proof, excluded from normal production bindings.
#[cfg(feature = "integration-proof")]
mod integration_proof;
