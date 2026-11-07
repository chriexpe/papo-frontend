//! Frozen v1 server canonicalization and identity; registry persistence belongs to storage.

use std::fmt;

use sha2::{Digest, Sha256};
use url::Url;

/// Rejected server address components, without retaining the supplied address.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServerAddressError {
    /// The WHATWG parser rejected the address.
    #[error("invalid server address")]
    InvalidUrl(#[source] url::ParseError),
    /// Only HTTP and HTTPS servers are supported.
    #[error("unsupported server scheme")]
    UnsupportedScheme,
    /// Credentials, including empty userinfo, are forbidden.
    #[error("server userinfo is forbidden")]
    Userinfo,
    /// Queries are forbidden, including an empty query.
    #[error("server query is forbidden")]
    Query,
    /// Fragments are forbidden, including an empty fragment.
    #[error("server fragment is forbidden")]
    Fragment,
}

/// A validated HTTP(S) address canonicalized by the frozen `ServerId` v1 rules.
///
/// Host serialization, ports and percent-encoding follow the WHATWG URL parser.
/// Path case is preserved and exactly one trailing slash is removed.
/// Debug output deliberately excludes the server address.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ServerAddress(String);

impl ServerAddress {
    /// Validates and canonicalizes an address without making a network request.
    ///
    /// # Errors
    /// Returns a typed rejection for malformed URLs, non-HTTP(S) schemes,
    /// userinfo, queries or fragments; no supplied address is kept in errors.
    pub fn parse(address: &str) -> Result<Self, ServerAddressError> {
        let parsed = Url::parse(address).map_err(ServerAddressError::InvalidUrl)?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(ServerAddressError::UnsupportedScheme);
        }
        if !parsed.username().is_empty() || parsed.password().is_some() || has_userinfo(address) {
            return Err(ServerAddressError::Userinfo);
        }
        if parsed.query().is_some() {
            return Err(ServerAddressError::Query);
        }
        if parsed.fragment().is_some() {
            return Err(ServerAddressError::Fragment);
        }
        let canonical = parsed.as_str();
        Ok(Self(
            canonical.strip_suffix('/').unwrap_or(canonical).to_owned(),
        ))
    }

    /// Returns the canonical address; callers must not write it to logs.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the frozen v1 identity of this canonical address.
    #[must_use]
    pub fn id(&self) -> ServerId {
        let mut digest = Sha256::new();
        digest.update(b"papo-server-id/v1\n");
        digest.update(self.0.as_bytes());
        let hash = digest.finalize();
        let mut bytes = [0; 16];
        bytes.copy_from_slice(&hash[..16]);
        ServerId(bytes)
    }
}

impl fmt::Debug for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ServerAddress([redacted])")
    }
}

/// First 16 bytes of SHA-256 over the v1 domain tag and canonical address.
///
/// Display is exactly 32 lowercase hexadecimal characters. These bytes are an
/// identity, not a UUID; changing canonicalization requires explicit re-keying.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ServerId([u8; 16]);

impl ServerId {
    /// Returns the 16 identity bytes in digest order.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for ServerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

fn has_userinfo(address: &str) -> bool {
    // WHATWG erases empty userinfo, so the parsed username alone is insufficient.
    let Some((_, remainder)) = address.split_once(':') else {
        return false;
    };
    let authority = remainder.trim_start_matches(['/', '\\', '\t', '\r', '\n']);
    authority
        .split(['/', '\\', '?', '#'])
        .next()
        .is_some_and(|part| part.contains('@'))
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
