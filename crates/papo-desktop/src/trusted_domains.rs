//! The trusted domains (`docs/design.md` §8, General › Trusted domains;
//! §9, Links): hosts whose links open without asking. This device's
//! preference, added from the link confirmation's box or the Trusted
//! domains page, removed there. A change shows at once and is written off
//! the UI thread; a failed write takes the list back to what the file
//! holds.
//!
//! Owns the list, its key in the settings file and what a typed domain
//! must be; what a press on a link does is `links`'.

use std::collections::BTreeSet;

use rune::prefs::Prefs;

use crate::device_settings::{SettingsFile, Stored, Write};

/// The settings file's key for the trusted domains.
const TRUSTED_KEY: &str = "trusted_domains";

/// The hosts whose links open without asking: this device's preference,
/// changed from the link confirmation's box and, with D6b, from General ›
/// Trusted domains. A change shows at once and is written off the UI
/// thread; a failed write takes the list back to what the file holds.
pub(crate) struct Trusted {
    stored: Stored<BTreeSet<String>>,
    file: SettingsFile,
}

impl Default for Trusted {
    /// None, kept nowhere: before the settings file is read, and in tests.
    fn default() -> Trusted {
        Trusted::read(&Prefs::memory())
    }
}

impl Trusted {
    /// The trusted domains in `prefs`, this device's settings file.
    pub(crate) fn read(prefs: &Prefs) -> Trusted {
        Trusted {
            stored: Stored::new(prefs.get(TRUSTED_KEY).unwrap_or_default()),
            file: SettingsFile::of(prefs),
        }
    }

    pub(crate) fn contains(&self, host: &str) -> bool {
        self.stored.shown().contains(host)
    }

    /// Trusts `host` from now on; the list to write now, unless a write is
    /// running (it then follows) or the host was trusted already.
    pub(crate) fn add(&mut self, host: String) -> Option<BTreeSet<String>> {
        let mut hosts = self.stored.shown().clone();
        hosts.insert(host);
        self.stored.change(hosts)
    }

    /// No longer trusts `host`; the list to write now, unless a write is
    /// running (it then follows) or the host was not trusted.
    pub(crate) fn remove(&mut self, host: &str) -> Option<BTreeSet<String>> {
        let mut hosts = self.stored.shown().clone();
        hosts.remove(host);
        self.stored.change(hosts)
    }

    /// Shows `hosts` as the list at once (Try again after a failed save);
    /// the list to write now, unless a write is running or nothing changed.
    pub(crate) fn replace(&mut self, hosts: BTreeSet<String>) -> Option<BTreeSet<String>> {
        self.stored.change(hosts)
    }

    /// The trusted hosts, in order.
    pub(crate) fn hosts(&self) -> &BTreeSet<String> {
        self.stored.shown()
    }

    /// The write of `hosts` landed (`ok`) or did not, which takes the list
    /// back to what the file holds; the list to write next.
    pub(crate) fn written(
        &mut self,
        hosts: BTreeSet<String>,
        ok: bool,
    ) -> Option<BTreeSet<String>> {
        self.stored.written(hosts, ok)
    }

    /// The write of the change waiting behind the running one, if any.
    pub(crate) fn pending_write(&self) -> Option<Write> {
        self.stored.pending().map(|hosts| self.write_of(hosts))
    }

    /// The write that saves `hosts`.
    pub(crate) fn write_of(&self, hosts: &BTreeSet<String>) -> Write {
        self.file
            .write(vec![(TRUSTED_KEY, serde_json::json!(hosts))])
    }
}

/// A host's id, the same in every run (FNV-1a of its name): its row's key,
/// which must not follow the list's order.
pub(crate) fn host_id(host: &str) -> u64 {
    host.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The host a typed domain names ("example.org", "Example.org.", or a
/// pasted address, `https://example.org/guide`), lower-cased; none for
/// text that names no host a link could go to.
pub(crate) fn typed_domain(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_whitespace) {
        return None;
    }
    let address = if text.contains("://") {
        text.to_owned()
    } else {
        format!("https://{text}")
    };
    let parsed = url::Url::parse(&address).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    // Hosts are named, not numbered, and have a dot: "localhost" or a bare
    // word is not a site.
    let url::Host::Domain(host) = parsed.host()? else {
        return None;
    };
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    host.contains('.').then_some(host)
}

#[cfg(test)]
mod tests {
    use super::{Trusted, typed_domain};

    #[test]
    fn a_typed_domain_is_its_host_lower_cased() {
        assert_eq!(typed_domain("example.org").as_deref(), Some("example.org"));
        assert_eq!(
            typed_domain("  Example.ORG. ").as_deref(),
            Some("example.org")
        );
        assert_eq!(
            typed_domain("https://docs.example.org/guide?x=1").as_deref(),
            Some("docs.example.org")
        );
    }

    #[test]
    fn text_that_names_no_site_is_refused() {
        for text in [
            "",
            "example",
            "two words.org",
            "ftp://example.org",
            "10.0.0.1",
            "localhost",
        ] {
            assert_eq!(typed_domain(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_host_is_added_once_and_removed() {
        let mut trusted = Trusted::default();
        let added = trusted.add("example.org".to_owned()).expect("a write");
        assert!(trusted.contains("example.org"));
        assert_eq!(trusted.written(added, true), None);
        assert_eq!(trusted.add("example.org".to_owned()), None, "already there");
        let removed = trusted.remove("example.org").expect("a write");
        assert!(!trusted.contains("example.org"));
        assert_eq!(trusted.written(removed, true), None);
    }
}
