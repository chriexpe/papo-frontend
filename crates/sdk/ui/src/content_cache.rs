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

use crate::message_content::MessageContent;

const MAX_CACHED_REVISIONS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ContentCacheKey {
    pub account: AccountId,
    pub channel: Uuid,
    pub message: Uuid,
}

#[derive(Clone)]
struct CacheEntry {
    message: Weak<Message>,
    content: Weak<MessageContent>,
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
    ) -> Option<(Arc<Message>, Arc<MessageContent>)> {
        let entry = self.entries.get(key)?;
        let cached_msg = entry.message.upgrade()?;
        let cached_content = entry.content.upgrade()?;
        same_or_ptr_eq(&cached_msg, current).then_some((cached_msg, cached_content))
    }

    fn insert(
        &mut self,
        key: ContentCacheKey,
        message: &Arc<Message>,
        content: &Arc<MessageContent>,
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
) {
    let mut cache = shared_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    cache.insert(key, message, content);
}

/// Returns a shared parsed [`MessageContent`] for `(key, message)`, parsing outside the lock on miss.
pub(crate) fn get_or_parse(
    key: Option<ContentCacheKey>,
    message: &Arc<Message>,
) -> Arc<MessageContent> {
    if let Some(key) = key {
        let hit = {
            let cache = shared_cache()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            cache.get_matching(&key, message)
        };
        if let Some((cached_msg, cached_content)) = hit {
            if !Arc::ptr_eq(&cached_msg, message) {
                refresh_revision(key, message, &cached_content);
            }
            return cached_content;
        }
    }

    let parsed = Arc::new(MessageContent::parse(
        message.content.as_deref().unwrap_or(""),
    ));

    if let Some(key) = key {
        let mut cache = shared_cache()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((cached_msg, cached_content)) = cache.get_matching(&key, message) {
            if !Arc::ptr_eq(&cached_msg, message) {
                cache.insert(key, message, &cached_content);
            }
            return cached_content;
        }
        cache.insert(key, message, &parsed);
    }

    parsed
}

#[cfg(test)]
mod tests {
    use super::*;
    use papo_sdk::ServerAddress;

    fn account(user_seed: u128, host: &str) -> AccountId {
        let server = ServerAddress::parse(host).expect("valid server URL").id();
        AccountId::new(server, Uuid::from_u128(user_seed))
    }

    fn message(id: u128, body: &str, edited_at: Option<&str>) -> Arc<Message> {
        Arc::new(Message {
            id: Some(Uuid::from_u128(id).to_string()),
            content: Some(body.into()),
            edited_at: edited_at.map(str::to_owned),
            ..Message::default()
        })
    }

    #[test]
    fn reuses_across_metadata_diffs_and_drops_when_weak_holders_go_away() {
        let key = ContentCacheKey {
            account: account(0xA1, "https://cache-reuse.invalid"),
            channel: Uuid::from_u128(0xC1),
            message: Uuid::from_u128(0xD1),
        };
        let msg_v1 = message(0xD1, "**hello** world", None);
        let parsed_v1 = get_or_parse(Some(key), &msg_v1);
        let again = get_or_parse(Some(key), &msg_v1);
        assert!(Arc::ptr_eq(&parsed_v1, &again));

        let mut reaction_updated = (*msg_v1).clone();
        reaction_updated.reactions = Some(Vec::new());
        let msg_v1_react = Arc::new(reaction_updated);
        let after_react = get_or_parse(Some(key), &msg_v1_react);
        assert!(Arc::ptr_eq(&parsed_v1, &after_react));

        let edited = message(0xD1, "**edited** world", Some("2026-01-01T00:01:00Z"));
        let parsed_v2 = get_or_parse(Some(key), &edited);
        assert!(!Arc::ptr_eq(&parsed_v1, &parsed_v2));

        let weak = Arc::downgrade(&parsed_v2);
        drop(parsed_v2);
        assert!(
            weak.upgrade().is_none(),
            "cache must never hold strong references"
        );
    }

    #[test]
    fn isolates_different_accounts_even_with_identical_ids_and_content() {
        let channel = Uuid::from_u128(0xC2);
        let msg_id = Uuid::from_u128(0xD2);
        let msg = message(0xD2, "same text", None);

        let a = get_or_parse(
            Some(ContentCacheKey {
                account: account(1, "https://iso.invalid"),
                channel,
                message: msg_id,
            }),
            &msg,
        );
        let b = get_or_parse(
            Some(ContentCacheKey {
                account: account(2, "https://iso.invalid"),
                channel,
                message: msg_id,
            }),
            &msg,
        );
        assert!(!Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn bounds_live_entries_without_unbounded_growth() {
        let acct = account(9, "https://bound.invalid");
        let channel = Uuid::from_u128(0xC3);
        let mut cache = CacheState::default();
        let mut retained = Vec::new();
        for index in 0..=(MAX_CACHED_REVISIONS * 2 + 32) {
            let msg_id = Uuid::from_u128(index as u128 + 1);
            let msg = message(index as u128 + 1, "entry", None);
            let parsed = Arc::new(MessageContent::parse("entry"));
            cache.insert(
                ContentCacheKey {
                    account: acct,
                    channel,
                    message: msg_id,
                },
                &msg,
                &parsed,
            );
            retained.push((msg, parsed));
        }
        assert!(cache.entries.len() <= MAX_CACHED_REVISIONS);
    }

    #[test]
    fn reclaims_dropped_entries_during_eviction() {
        let acct = account(10, "https://reclaim.invalid");
        let channel = Uuid::from_u128(0xC4);
        let mut cache = CacheState::default();
        let first_id = Uuid::from_u128(1);
        let first_msg = message(1, "drop me", None);
        let first_parsed = Arc::new(MessageContent::parse("drop me"));
        cache.insert(
            ContentCacheKey {
                account: acct,
                channel,
                message: first_id,
            },
            &first_msg,
            &first_parsed,
        );
        let mut kept = Vec::new();
        for index in 2..=MAX_CACHED_REVISIONS {
            let msg_id = Uuid::from_u128(index as u128);
            let msg = message(index as u128, "keep", None);
            let parsed = Arc::new(MessageContent::parse("keep"));
            cache.insert(
                ContentCacheKey {
                    account: acct,
                    channel,
                    message: msg_id,
                },
                &msg,
                &parsed,
            );
            kept.push((msg, parsed));
        }
        drop(first_msg);
        drop(first_parsed);

        let extra_id = Uuid::from_u128(99_999);
        let extra_msg = message(99_999, "extra", None);
        let extra_parsed = Arc::new(MessageContent::parse("extra"));
        cache.insert(
            ContentCacheKey {
                account: acct,
                channel,
                message: extra_id,
            },
            &extra_msg,
            &extra_parsed,
        );
        assert_eq!(cache.entries.len(), MAX_CACHED_REVISIONS);
        assert_eq!(cache.evictions_since_sweep, 0);
        assert!(cache.entries.contains_key(&ContentCacheKey {
            account: acct,
            channel,
            message: extra_id,
        }));
    }
}
