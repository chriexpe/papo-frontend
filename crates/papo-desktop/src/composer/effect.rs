//! What the composer asks of the shell ([`Effect`]) and what Send did
//! ([`Sent`]). The shell carries effects out against the SDK
//! (`shell/composer.rs`).

use papo_sdk::Uuid;

use crate::timeline::TimelineKey;

/// What the shell does for the composer.
#[derive(Debug, PartialEq)]
pub(crate) enum Effect {
    /// Read the channel's draft from the SDK.
    Load(TimelineKey),
    /// Save `text` (its wire form, mentions as tokens) as the channel's
    /// draft, with the people it mentions. `seq` orders writes: one older
    /// than the last written for the channel is dropped.
    Save {
        key: TimelineKey,
        text: String,
        mentions: Vec<Uuid>,
        seq: u64,
    },
    /// Queue `text` (its wire form) as a new message with local identity
    /// `local`, then clear the stored draft unless a newer save (`seq`)
    /// landed first.
    Send {
        key: TimelineKey,
        text: String,
        local: Uuid,
        seq: u64,
    },
    /// Tell the channel the user is typing.
    ReportTyping(TimelineKey),
    /// Call the composer's poll again in this many seconds.
    WakeAfter(f64),
}

/// What pressing Send (or Enter) did.
#[derive(Debug, PartialEq)]
pub(crate) enum Sent {
    /// Nothing to send: the field is empty, blank, or still loading.
    Nothing,
    /// Over [`MAX_MESSAGE_CHARS`](super::MAX_MESSAGE_CHARS): the field keeps the text.
    TooLong,
    /// The text left the field; the effect queues it.
    Queued(Effect),
}
