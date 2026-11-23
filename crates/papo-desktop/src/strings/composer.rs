//! The composer's strings and those of your unsent messages: the field, Send,
//! the length and send-failure lines, the pending and failed marks, their
//! menu and the "Send again?" confirmation. The catalog entries are
//! `composer.*` and `outgoing.*`.

use super::{I18N, Key, PluralKey, catalog};

/// What assistive technology announces for the message field.
pub(crate) fn composer_label() -> &'static str {
    text!("composer.label")
}

/// The hint in the empty field: "Message #general".
pub(crate) fn composer_placeholder(channel: &str) -> String {
    template!("composer.placeholder", "channel" => channel)
}

/// The empty field's prompt in a direct conversation: "Message @Lia".
pub(crate) fn composer_placeholder_direct(name: &str) -> String {
    template!("composer.placeholder_direct", "name" => name)
}

/// Send's label.
pub(crate) fn composer_send() -> &'static str {
    text!("composer.send")
}

/// The line over the field after a send the SDK did not take.
pub(crate) fn composer_send_failed() -> &'static str {
    text!("composer.send_failed")
}

/// The line over the field while it is too long to send.
pub(crate) fn composer_too_long(over: u64) -> String {
    plural!("composer.too_long", over)
}

/// A message waiting to be sent.
pub(crate) fn outgoing_pending() -> &'static str {
    text!("outgoing.pending")
}

/// A message the server refused.
pub(crate) fn outgoing_failed() -> &'static str {
    text!("outgoing.failed")
}

/// A message whose send was cut off: it may have arrived.
pub(crate) fn outgoing_unknown() -> &'static str {
    text!("outgoing.unknown")
}

/// What assistive technology announces for the ⚠ that opens the menu:
/// "Not sent. Options".
pub(crate) fn outgoing_options(state: &str) -> String {
    template!("outgoing.options", "state" => state)
}

/// The menu's Retry.
pub(crate) fn outgoing_retry() -> &'static str {
    text!("outgoing.retry")
}

/// The menu's Discard.
pub(crate) fn outgoing_discard() -> &'static str {
    text!("outgoing.discard")
}

/// "Send again?", before retrying a message that may have arrived.
pub(crate) fn resend_title() -> &'static str {
    text!("outgoing.resend_title")
}

/// Why it asks.
pub(crate) fn resend_body() -> &'static str {
    text!("outgoing.resend_body")
}

/// The confirmation's Cancel, the default.
pub(crate) fn resend_cancel() -> &'static str {
    text!("outgoing.resend_cancel")
}

/// The confirmation's "Send again".
pub(crate) fn resend_confirm() -> &'static str {
    text!("outgoing.resend_confirm")
}

/// The @ picker, for assistive technology.
pub(crate) fn mention_picker() -> &'static str {
    text!("composer.mention_picker")
}

/// What @everyone does, beside it in the @ picker.
pub(crate) fn mention_everyone_hint() -> &'static str {
    text!("composer.mention_everyone")
}
