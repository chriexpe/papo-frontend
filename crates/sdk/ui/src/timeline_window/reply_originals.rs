//! What each reply's original is, from the window's own messages first and otherwise from the
//! canonical lookup: `HistorySnapshot::locate` for the first original the window does not show,
//! then `HistorySnapshot::locator` (the same answers after one indexing pass) once a second
//! one makes the index cheaper than another scan. Measured on 10,000 held messages: a scan
//! costs up to 0.22 ms, the index 0.34 ms to build. Owns no cache and fetches nothing: an original
//! history does not hold is `Unknown`, never a deletion. Drawing the reply is the client's.

use std::cell::{Cell, OnceCell};
use std::collections::HashMap;
use std::sync::Arc;

use papo_sdk::{
    AccountId, HistorySnapshot, InvalidMessageKey, LocatedMessage, Message, MessageLocator, Uuid,
};

use crate::{
    content_cache::{self, ContentCacheKey},
    message_content::MessageContent,
    timeline_item::ReplyOriginal,
};

/// Resolves originals for one projection of one window.
pub(crate) struct Originals<'a> {
    raw: &'a HistorySnapshot,
    cache_scope: Option<(AccountId, Uuid)>,
    /// The window's messages seen so far. An original is normally older than its reply, so a
    /// window projected oldest first has usually met it; one that has not (a skewed clock) is
    /// still found in history.
    shown: HashMap<Uuid, (Arc<Message>, Arc<MessageContent>)>,
    /// History's index, built on the second original the window does not show.
    locator: OnceCell<MessageLocator<'a>>,
    /// Originals looked up in history so far.
    misses: Cell<u32>,
}

impl<'a> Originals<'a> {
    pub(crate) fn new(raw: &'a HistorySnapshot, cache_scope: Option<(AccountId, Uuid)>) -> Self {
        Self {
            raw,
            cache_scope,
            shown: HashMap::new(),
            locator: OnceCell::new(),
            misses: Cell::new(0),
        }
    }

    /// Records a message this window shows, so its replies share its `Arc`s.
    pub(crate) fn show(&mut self, id: Uuid, message: &Arc<Message>, content: &Arc<MessageContent>) {
        self.shown
            .insert(id, (Arc::clone(message), Arc::clone(content)));
    }

    /// The original of a message whose wire `reply_to` is `reply_to`: none when it replies to
    /// nothing (absent or empty), `Unknown` when the ID does not parse.
    /// # Errors
    /// Returns `InvalidMessageKey` if a held message lacks a valid key.
    pub(crate) fn of_wire(
        &self,
        reply_to: Option<&str>,
    ) -> Result<Option<ReplyOriginal>, InvalidMessageKey> {
        let Some(raw_id) = reply_to.filter(|id| !id.is_empty()) else {
            return Ok(None);
        };
        match raw_id.parse::<Uuid>() {
            Ok(id) => self.of(id).map(Some),
            Err(_) => Ok(Some(ReplyOriginal::Unknown)),
        }
    }

    /// The original with backend ID `id`.
    /// # Errors
    /// Returns `InvalidMessageKey` if a held message lacks a valid key.
    pub(crate) fn of(&self, id: Uuid) -> Result<ReplyOriginal, InvalidMessageKey> {
        if let Some((message, content)) = self.shown.get(&id) {
            return Ok(ReplyOriginal::Held {
                id,
                message: Arc::clone(message),
                content: Arc::clone(content),
            });
        }
        Ok(match self.locate(id)? {
            LocatedMessage::Held { index, .. } => match self.raw.messages.get(index) {
                Some(message) => {
                    let key = self.cache_scope.map(|(account, channel)| ContentCacheKey {
                        account,
                        channel,
                        message: id,
                    });
                    ReplyOriginal::Held {
                        id,
                        message: Arc::clone(message),
                        content: content_cache::get_or_parse(key, message),
                    }
                }
                None => ReplyOriginal::Unknown,
            },
            LocatedMessage::Deleted => ReplyOriginal::Deleted,
            LocatedMessage::Unknown => ReplyOriginal::Unknown,
        })
    }
}

impl Originals<'_> {
    /// History's answer for `id`: one scan for the first miss, the index after that.
    fn locate(&self, id: Uuid) -> Result<LocatedMessage, InvalidMessageKey> {
        let misses = self.misses.get().saturating_add(1);
        self.misses.set(misses);
        if misses == 1 {
            return self.raw.locate(id);
        }
        let raw = self.raw;
        self.locator.get_or_init(|| raw.locator()).locate(id)
    }
}
