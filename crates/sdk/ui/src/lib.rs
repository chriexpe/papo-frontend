//! Observable Papo UI services belong here; transport and durable state belong to `papo-sdk`.
//! Platform widgets, navigation and translated text belong to client applications.

/// Integration-only sample diffs, never exposed by production services.
#[cfg(feature = "integration-proof")]
pub mod integration_proof;
