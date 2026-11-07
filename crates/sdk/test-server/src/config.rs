//! Owns deterministic scenario configuration; leaves execution and wire handling to the server.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Configuration for one fresh in-memory server; no production data is needed.
///
/// Secrets and event bodies deliberately have no `Debug` implementation.
#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Display name returned by public discovery.
    pub server_name: String,
    /// Whether discovery reports a server; `false` models bootstrap.
    pub server_exists: bool,
    /// A private server requires a temporary cookie before registration or login.
    pub public: bool,
    /// Private-server password, supplied at runtime rather than in fixtures.
    pub server_password: Option<String>,
    /// Minimum registration password length in Unicode scalar values.
    pub min_password_chars: usize,
    /// Maximum password length in Unicode scalar values.
    pub max_password_chars: usize,
    /// Maximum username length in Unicode scalar values.
    pub max_username_chars: usize,
    /// Exact allowed browser origins; non-browser clients omit Origin.
    pub allowed_origins: Vec<String>,
    /// REST overrides consumed once, in order, by the next matching request.
    pub rest: Vec<RestStep>,
    /// Steps replayed independently after each authenticated gateway connection.
    pub gateway: Vec<GatewayStep>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server_name: "Papo test backend".into(),
            server_exists: true,
            public: true,
            server_password: None,
            min_password_chars: 8,
            max_password_chars: 64,
            max_username_chars: 16,
            allowed_origins: Vec::new(),
            rest: Vec::new(),
            gateway: Vec::new(),
        }
    }
}

/// A one-shot REST fault or delay, consumed in configuration order.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestStep {
    /// Exact uppercase HTTP method.
    pub method: String,
    /// Exact request path, excluding the query.
    pub path: String,
    /// Delay before handling the request, in milliseconds.
    #[serde(default)]
    pub delay_ms: u64,
    /// Optional injected RFC 7807 failure instead of running the handler.
    #[serde(default)]
    pub failure: Option<Failure>,
}

/// An injected REST failure, with the backend's problem response shape.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Failure {
    /// HTTP error status in the range 400 through 599.
    pub status: u16,
    /// Backend problem slug, appended to `/errors/`.
    pub slug: String,
}

/// Ordered gateway actions, without any implicit real-time or random decisions.
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum GatewayStep {
    /// Wait before the next script step while still accepting client frames.
    Delay {
        /// Delay duration in milliseconds.
        delay_ms: u64,
    },
    /// Send an arbitrary JSON event; list order permits duplicates or reordering.
    Event {
        /// Runtime-supplied event payload; never logged or included in fixtures.
        event: Value,
    },
    /// Close normally after all preceding steps.
    Disconnect,
}

#[cfg(test)]
mod tests {
    //! Owns pure configuration tests; leaves network execution to integration tests.

    use super::*;

    #[test]
    fn omitted_fields_use_backend_auth_limits() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(
            (
                config.min_password_chars,
                config.max_password_chars,
                config.max_username_chars
            ),
            (8, 64, 16)
        );
    }

    #[test]
    fn unknown_configuration_fields_are_rejected() {
        assert!(serde_json::from_str::<Config>(r#"{"unknown":true}"#).is_err());
    }
}
