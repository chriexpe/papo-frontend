//! Converts SDK ICE server configuration into the URI form accepted by webrtcbin.
//! No address or credential is included in diagnostics.

use papo_sdk::IceServerConfig;
use std::{fmt, fmt::Write as _};

#[derive(Default)]
pub(crate) struct IceConfig {
    pub stun: Option<String>,
    pub turn: Vec<String>,
}

impl fmt::Debug for IceConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IceConfig")
            .field("has_stun", &self.stun.is_some())
            .field("turn_count", &self.turn.len())
            .finish()
    }
}

impl IceConfig {
    pub(crate) fn from_servers(servers: &[IceServerConfig]) -> Self {
        let mut config = Self::default();
        for server in servers {
            for url in &server.urls {
                let Some((scheme, address)) = url.trim().split_once(':') else {
                    continue;
                };
                let address = address.strip_prefix("//").unwrap_or(address);
                match scheme {
                    "stun" if config.stun.is_none() => {
                        config.stun = Some(format!("stun://{address}"));
                    }
                    "turn" | "turns" => {
                        if let (Some(user), Some(secret)) = (&server.username, &server.credential) {
                            config.turn.push(format!(
                                "{scheme}://{}:{}@{address}",
                                escape(user),
                                escape(secret)
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
        config
    }
}

fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webrtc_stun_uri_has_two_slashes() {
        for url in ["stun:example.invalid:3478", "stun://example.invalid:3478"] {
            let config = IceConfig::from_servers(&[IceServerConfig {
                urls: vec![url.into()],
                username: None,
                credential: None,
            }]);
            assert_eq!(config.stun.as_deref(), Some("stun://example.invalid:3478"));
        }
    }

    #[test]
    fn escapes_turn_credentials_and_redacts_uris_from_debug() {
        let config = IceConfig::from_servers(&[IceServerConfig {
            urls: vec!["turn:example.invalid:3478?transport=udp".into()],
            username: Some("expiry:fixture".into()),
            credential: Some("ab+cd/ef=".into()),
        }]);
        assert_eq!(
            config.turn,
            ["turn://expiry%3Afixture:ab%2Bcd%2Fef%3D@example.invalid:3478?transport=udp"]
        );
        let debug = format!("{config:?}");
        for secret in ["expiry", "ab%2Bcd", "example.invalid"] {
            assert!(!debug.contains(secret));
        }
    }

    #[test]
    fn skips_turn_without_credentials() {
        let config = IceConfig::from_servers(&[IceServerConfig {
            urls: vec!["turn:example.invalid:3478".into()],
            username: None,
            credential: None,
        }]);
        assert!(config.turn.is_empty());
    }
}
