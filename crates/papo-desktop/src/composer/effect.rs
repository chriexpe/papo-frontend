//! What the composer asks of the shell ([`Effect`]) and what Send did
//! ([`Sent`]). The shell carries effects out against the SDK
//! (`shell/composer.rs`).

use papo_sdk::Uuid;

use super::tray::Tile;
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
        reply_to: Option<Uuid>,
        /// The tray's staged tiles, in order; a tile still copying waits
        /// for its own save.
        tiles: Vec<Tile>,
        seq: u64,
    },
    /// Queue `text` (its wire form) and `tiles` as a new message with local
    /// identity `local`, replying to `reply_to` when given, then clear the
    /// stored draft unless a newer save (`seq`) landed first. A tile still
    /// copying goes as it is: the message waits as Preparing.
    Send {
        key: TimelineKey,
        text: String,
        reply_to: Option<Uuid>,
        tiles: Vec<Tile>,
        local: Uuid,
        seq: u64,
        /// The emoji it holds, counted toward Frequently used and the quick reactions
        /// (`docs/design.md` §16, "One ranking") once it is queued.
        emoji: Vec<String>,
    },
    /// Queue an edit of your message `id` to `text` (its wire form), with
    /// local identity `local`. The stored draft is not touched.
    Edit {
        key: TimelineKey,
        id: Uuid,
        text: String,
        local: Uuid,
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
    /// An edit left the field; `edit` queues it, and `save` stores the
    /// draft that came back when it holds something not yet stored.
    Edited { edit: Effect, save: Option<Effect> },
    /// An edit emptied of text: the message is to be deleted instead, once
    /// the person confirms (`docs/design.md` §11). The edit stays open.
    DeleteInstead { key: TimelineKey, id: Uuid },
}
