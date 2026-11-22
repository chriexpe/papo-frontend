//! The reading controls' strings (`docs/design.md` §6, Reading controls):
//! the new-messages divider, the skipped block's edges, ⌛ and ↓, and the
//! arrivals count. The catalog entries are `reading.*`.

use super::{I18N, Key, PluralKey, catalog};

/// The divider's chip, before its ↓: "New messages".
pub(crate) fn new_messages() -> &'static str {
    text!("reading.new_messages")
}

/// What assistive technology announces for the divider.
pub(crate) fn new_messages_spoken() -> &'static str {
    text!("reading.new_messages_spoken")
}

/// A skipped block's chips, before their ↓ or ↑: "Unread".
pub(crate) fn unread() -> &'static str {
    text!("reading.unread")
}

/// What assistive technology announces for "Unread" and its ↓.
pub(crate) fn unread_below_spoken() -> &'static str {
    text!("reading.unread_below_spoken")
}

/// What assistive technology announces for "Unread" and its ↑.
pub(crate) fn unread_above_spoken() -> &'static str {
    text!("reading.unread_above_spoken")
}

/// ⌛'s label.
pub(crate) fn hourglass() -> &'static str {
    text!("reading.hourglass")
}

/// ↓'s label when it goes to the first new message.
pub(crate) fn first_new() -> &'static str {
    text!("reading.first_new")
}

/// ↓'s label when it goes to the newest message.
pub(crate) fn latest() -> &'static str {
    text!("reading.latest")
}

/// ↓'s label when it clears the skipped block.
pub(crate) fn clear_block() -> &'static str {
    text!("reading.clear_block")
}

/// The pill shown while ⌛ pages toward the skipped block.
pub(crate) fn seeking() -> &'static str {
    text!("reading.seeking")
}

/// Stops ⌛'s paging.
pub(crate) fn seek_cancel() -> &'static str {
    text!("reading.cancel")
}

/// "3 new messages": the arrivals count, spoken.
pub(crate) fn arrivals(count: u64) -> String {
    plural!("reading.arrivals", count)
}

/// ↓'s label with its arrivals: "{action}, {arrivals}".
pub(crate) fn with_arrivals(action: &str, arrivals: &str) -> String {
    template!("reading.labelled", "action" => action, "arrivals" => arrivals)
}
