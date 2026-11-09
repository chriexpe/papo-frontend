//! Owns opaque cookie snapshots and in-memory cookie scope; secret storage and generations belong to the SDK.

use crate::Error;
use cookie_store::{Cookie, CookieExpiration, CookieStore};
use reqwest::header::HeaderValue;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, ops::Deref, sync::RwLock};
use url::Url;

const MAX_SNAPSHOT_BYTES: usize = 64 * 1024;
const MAX_COOKIES: usize = 64;
const MAX_COOKIE_BYTES: usize = 8192;

/// Address-bound credential bytes for an OS secret store, never a cache or configuration file.
///
/// Deliberately implements neither Debug, Display nor serde traits. Exposure is explicit;
/// the caller must not log or persist the bytes outside its secret-store boundary.
///
/// ```compile_fail
/// fn diagnostic<T: std::fmt::Debug>() {}
/// diagnostic::<papo_api::CredentialSnapshot>();
/// ```
/// ```compile_fail
/// fn diagnostic<T: std::fmt::Display>() {}
/// diagnostic::<papo_api::CredentialSnapshot>();
/// ```
/// ```compile_fail
/// fn serialize<T: serde::Serialize>() {}
/// serialize::<papo_api::CredentialSnapshot>();
/// ```
/// ```compile_fail
/// fn deserialize<T: serde::de::DeserializeOwned>() {}
/// deserialize::<papo_api::CredentialSnapshot>();
/// ```
pub struct CredentialSnapshot {
    bytes: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    version: u8,
    base: String,
    cookies: Vec<String>,
}

impl CredentialSnapshot {
    /// Validates a versioned snapshot read from a secret store without exposing diagnostics.
    /// Expired cookies remain expired and are omitted on restoration, never renewed.
    /// # Errors
    /// Returns `Error::Credentials` for invalid encoding, version, bounds or cookie scope.
    pub fn import(bytes: &[u8]) -> Result<Self, Error> {
        decode(bytes)?;
        Ok(Self {
            bytes: bytes.to_vec(),
        })
    }

    /// Explicitly exposes secret bytes solely for transfer to a secret store.
    #[must_use]
    pub fn expose_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn restore(&self, base: &Url, loopback: bool) -> Result<CookieJar, Error> {
        let (scope, store) = decode(&self.bytes)?;
        if scope != *base {
            return Err(Error::Credentials);
        }
        Ok(CookieJar {
            store: RwLock::new(store),
            loopback,
        })
    }
}

/// Validates structure and address binding without parsing cookies or consulting a clock.
fn decode_envelope(bytes: &[u8]) -> Result<(Url, Vec<String>), Error> {
    if bytes.is_empty() || bytes.len() > MAX_SNAPSHOT_BYTES {
        return Err(Error::Credentials);
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| Error::Credentials)?;
    if envelope.version != 1 || envelope.cookies.len() > MAX_COOKIES {
        return Err(Error::Credentials);
    }
    let base =
        crate::rest::validated_base(&envelope.base, false).map_err(|_| Error::Credentials)?;
    if base.as_str() != envelope.base {
        return Err(Error::Credentials);
    }
    Ok((base, envelope.cookies))
}

fn decode(bytes: &[u8]) -> Result<(Url, CookieStore), Error> {
    let (base, cookies) = decode_envelope(bytes)?;
    let mut seen = HashSet::new();
    let mut store = CookieStore::default();
    for raw in cookies {
        let cookie = parse_cookie(&raw, &base).ok_or(Error::Credentials)?;
        if normalized(&cookie) != raw {
            return Err(Error::Credentials);
        }
        let key = (
            String::from(&cookie.domain),
            String::from(&cookie.path),
            cookie.name().to_owned(),
        );
        if !seen.insert(key) {
            return Err(Error::Credentials);
        }
        if !cookie.is_expired() {
            store
                .insert(cookie, &base)
                .map_err(|_| Error::Credentials)?;
        }
    }
    Ok((base, store))
}

fn parse_cookie(raw: &str, url: &Url) -> Option<Cookie<'static>> {
    if raw.len() > MAX_COOKIE_BYTES || raw.bytes().any(|b| !b.is_ascii() || b.is_ascii_control()) {
        return None;
    }
    let cookie = Cookie::parse(raw.to_owned(), url).ok()?.into_owned();
    if cookie.name().is_empty()
        || !cookie
            .name()
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
        || !cookie
            .value()
            .bytes()
            .all(|b| matches!(b, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e))
    {
        return None;
    }
    Some(cookie)
}

fn normalized(cookie: &Cookie<'_>) -> String {
    let mut raw = cookie.deref().clone().into_owned();
    raw.set_path(String::from(&cookie.path));
    raw.set_max_age(None);
    match cookie.expires {
        CookieExpiration::AtUtc(expiry) => raw.set_expires(expiry),
        CookieExpiration::SessionEnd => raw.set_expires(None),
    }
    raw.to_string()
}

/// A provider with no secret-bearing logging or diagnostic implementation.
pub(crate) struct CookieJar {
    store: RwLock<CookieStore>,
    loopback: bool,
}

impl CookieJar {
    pub(crate) fn new(loopback: bool) -> Self {
        Self {
            store: RwLock::new(CookieStore::default()),
            loopback,
        }
    }

    pub(crate) fn snapshot(&self, base: &Url) -> Result<CredentialSnapshot, Error> {
        let store = self.store.read().map_err(|_| Error::Credentials)?;
        let cookies = store.iter_unexpired().map(normalized).collect::<Vec<_>>();
        let bytes = serde_json::to_vec(&Envelope {
            version: 1,
            base: base.to_string(),
            cookies,
        })
        .map_err(|_| Error::Credentials)?;
        CredentialSnapshot::import(&bytes)
    }
}

impl reqwest::cookie::CookieStore for CookieJar {
    fn set_cookies(&self, headers: &mut dyn Iterator<Item = &HeaderValue>, url: &Url) {
        let Ok(mut store) = self.store.write() else {
            return;
        };
        for header in headers {
            if let Some(cookie) = header.to_str().ok().and_then(|raw| parse_cookie(raw, url)) {
                // Avoid cookie_store's convenience method, which logs cookie material.
                let _ = store.insert(cookie, url);
            }
        }
    }

    fn cookies(&self, url: &Url) -> Option<HeaderValue> {
        let store = self.store.read().ok()?;
        let mut scope = url.clone();
        if self.loopback && scope.scheme() == "http" {
            // The explicit loopback constructor relaxes transmission, not stored Secure metadata.
            scope.set_scheme("https").ok()?;
        }
        let mut matching = store.matches(&scope);
        matching.sort_by_key(|cookie| std::cmp::Reverse(cookie.path.len()));
        let value = matching
            .into_iter()
            .filter(|cookie| !cookie.secure().unwrap_or(false) || scope.scheme() == "https")
            .map(|cookie| format!("{}={}", cookie.name(), cookie.value()))
            .collect::<Vec<_>>()
            .join("; ");
        if value.is_empty() {
            return None;
        }
        let mut header = HeaderValue::from_str(&value).ok()?;
        header.set_sensitive(true);
        Some(header)
    }
}

#[cfg(test)]
mod tests;
