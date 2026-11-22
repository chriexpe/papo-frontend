//! The open-link confirmation's strings. The catalog entries are `link.*`.

use super::{I18N, Key, catalog};

/// The confirmation's title, and what assistive technology announces.
pub(crate) fn link_title() -> &'static str {
    text!("link.title")
}

/// "Always open links from {host}".
pub(crate) fn link_always(host: &str) -> String {
    template!("link.always", "host" => host)
}

/// Closes the confirmation without opening the link.
pub(crate) fn link_cancel() -> &'static str {
    text!("link.cancel")
}

/// Opens the link.
pub(crate) fn link_open() -> &'static str {
    text!("link.open")
}
