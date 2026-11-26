//! Account-isolated, bounded `Weak`-only cache sharing parsed [`MessageContent`] across windows.
//!
//! Entries hold only weak references to wire [`Message`] and parsed [`MessageContent`] allocations
//! so closed timelines and dropped snapshots release memory immediately. Only brief map lookups
//! and insertions touch the mutex; Markdown parsing always executes outside the lock.

use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex, OnceLock, Weak},
};

use papo_sdk::{AccountId, Message, Uuid};

use crate::message_content::{CustomEmojis, MessageContent};

const MAX_CACHED_REVISIONS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ContentCacheKey {
    pub account: AccountId,
    pub channel: Uuid,
    pub message: Uuid,
}

/// Where parsed content is shared (`cache`: the account and channel, none to share nothing)
/// and what its `:name:` resolves against.
#[derive(Clone, Copy)]
pub(crate) struct ContentScope<'a> {
    pub cache: Option<(AccountId, Uuid)>,
    pub emojis: &'a CustomEmojis,
}

impl ContentScope<'_> {
    /// The cache key of message `message`, when content is shared.
    pub fn key(&self, message: Uuid) -> Option<ContentCacheKey> {
        self.cache.map(|(account, channel)| ContentCacheKey {
            account,
            channel,
            message,
        })
    }
}

#[derive(Clone)]
struct CacheEntry {
    message: Weak<Message>,
    content: Weak<MessageContent>,
    /// The custom emoji revision the content was resolved against.
    emojis: u64,
    seq: u64,
}

impl CacheEntry {
    fn is_live(&self) -> bool {
        self.message.strong_count() > 0 && self.content.strong_count() > 0
    }
}

#[derive(Default)]
struct CacheState {
    entries: HashMap<ContentCacheKey, CacheEntry>,
    order: VecDeque<(ContentCacheKey, u64)>,
    next_seq: u64,
    evictions_since_sweep: usize,
}

impl CacheState {
    fn get_matching(
        &self,
        key: &ContentCacheKey,
        current: &Arc<Message>,
        emojis: u64,
    ) -> Option<(Arc<Message>, Arc<MessageContent>)> {
        let entry = self
            .entries
            .get(key)
            .filter(|entry| entry.emojis == emojis)?;
        let cached_msg = entry.message.upgrade()?;
        let cached_content = entry.content.upgrade()?;
        same_or_ptr_eq(&cached_msg, current).then_some((cached_msg, cached_content))
    }

    fn insert(
        &mut self,
        key: ContentCacheKey,
        message: &Arc<Message>,
        content: &Arc<MessageContent>,
        emojis: u64,
    ) {
        if !self.entries.contains_key(&key) && self.entries.len() >= MAX_CACHED_REVISIONS {
            self.evict_stale_or_oldest();
        }
        self.next_seq = self.next_seq.wrapping_add(1);
        let seq = self.next_seq;
        self.entries.insert(
            key,
            CacheEntry {
                message: Arc::downgrade(message),
                content: Arc::downgrade(content),
                emojis,
                seq,
            },
        );
        self.order.push_back((key, seq));
        // Compact queue entries from repeated updates so order stays bounded too.
        if self.order.len() > MAX_CACHED_REVISIONS * 2 {
            self.order.retain(|(queued_key, queued_seq)| {
                self.entries
                    .get(queued_key)
                    .is_some_and(|e| e.seq == *queued_seq)
            });
        }
    }

    fn evict_stale_or_oldest(&mut self) {
        while self.entries.len() >= MAX_CACHED_REVISIONS {
            if self.evictions_since_sweep >= MAX_CACHED_REVISIONS {
                // Amortize full dead-entry sweeps across MAX_CACHED_REVISIONS evictions
                // instead of scanning all cached entries on every insertion above capacity.
                self.entries.retain(|_, entry| entry.is_live());
                self.evictions_since_sweep = 0;
                if self.entries.len() < MAX_CACHED_REVISIONS {
                    break;
                }
            }
            let Some((oldest_key, oldest_seq)) = self.order.pop_front() else {
                self.entries.clear();
                break;
            };
            let Some(entry) = self.entries.get(&oldest_key) else {
                continue;
            };
            if entry.seq != oldest_seq {
                continue;
            }
            let was_live = entry.is_live();
            self.entries.remove(&oldest_key);
            if was_live {
                self.evictions_since_sweep += 1;
            }
        }
    }
}

fn shared_cache() -> &'static Mutex<CacheState> {
    static CACHE: OnceLock<Mutex<CacheState>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(CacheState::default()))
}

pub(crate) fn same_revision(a: &Message, b: &Message) -> bool {
    a.content == b.content && a.edited_at == b.edited_at
}

fn same_or_ptr_eq(a: &Arc<Message>, b: &Arc<Message>) -> bool {
    Arc::ptr_eq(a, b) || same_revision(a, b)
}

/// Refreshes the cached weak `Message` pointer when an existing `content` `Arc` was reused
/// across a metadata-only update (e.g. reactions or embeds) in the current window.
pub(crate) fn refresh_revision(
    key: ContentCacheKey,
    message: &Arc<Message>,
    content: &Arc<MessageContent>,
    emojis: &CustomEmojis,
) {
    let mut cache = shared_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache.insert(key, message, content, emojis.revision());
}

/// Returns a shared parsed [`MessageContent`] for `(key, message)` resolved against `emojis`,
/// parsing outside the lock on miss. Content resolved against another emoji revision is a miss.
pub(crate) fn get_or_parse(
    key: Option<ContentCacheKey>,
    message: &Arc<Message>,
    emojis: &CustomEmojis,
) -> Arc<MessageContent> {
    let revision = emojis.revision();
    if let Some(key) = key {
        let hit = {
            let cache = shared_cache()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.get_matching(&key, message, revision)
        };
        if let Some((cached_msg, cached_content)) = hit {
            if !Arc::ptr_eq(&cached_msg, message) {
                refresh_revision(key, message, &cached_content, emojis);
            }
            return cached_content;
        }
    }

    let parsed = Arc::new(emojis.parse(message.content.as_deref().unwrap_or("")));

    if let Some(key) = key {
        let mut cache = shared_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((cached_msg, cached_content)) = cache.get_matching(&key, message, revision) {
            if !Arc::ptr_eq(&cached_msg, message) {
                cache.insert(key, message, &cached_content, revision);
            }
            return cached_content;
        }
        cache.insert(key, message, &parsed, revision);
    }

    parsed
}

#[cfg(test)]
mod tests;
