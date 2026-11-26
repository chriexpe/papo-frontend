//! Custom emoji images as content draws them (`docs/design.md` §9, Emoji): each emoji's image
//! decoded once, to fit the largest box it is drawn in (jumbo at the window's scale), and kept
//! while a view still wants it, up to [`MAX_KEPT`] emoji.
//!
//! Pure: it decides which images to fetch and keeps what landed. The bytes are the SDK's
//! (`MediaService::fetch` on the emoji's source; the catalog lists them into the store), the
//! fetching and decoding run off the UI thread in `shell::emoji_images`, and the drawing is
//! `timeline::content`'s and the composer's.

use std::collections::HashMap;
use std::sync::Arc;

use papo_sdk::{AccountId, CustomEmoji, MediaSource, Uuid};
use rune::Pixels;

mod drawing;
pub(crate) use drawing::{EmojiPlace, emoji_square, inline_image, with_images};

/// The most decoded emoji kept: at most `DECODE_MAX_PX`² × 4 bytes each, so about 26 MiB.
const MAX_KEPT: usize = 256;

/// The longest side an emoji is decoded to, in device pixels: jumbo (3.4 × the largest text
/// size, 16 pt) at a scale of two and a half.
pub(crate) const DECODE_MAX_PX: u32 = 160;

/// One account's emoji.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct EmojiKey {
    pub(crate) account: AccountId,
    pub(crate) id: Uuid,
}

/// What a view draws for an emoji now.
#[derive(Clone, Debug)]
pub(crate) enum EmojiLook {
    /// Not here yet: its square is reserved.
    Loading,
    /// Its image, decoded; `aspect` is width over height.
    Ready { pixels: Pixels, aspect: f32 },
    /// It could not be fetched or decoded: its `:name:` shows as text (§9).
    Failed,
}

impl PartialEq for EmojiLook {
    /// The same look: two images are the same when they are the same pixels.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (EmojiLook::Loading, EmojiLook::Loading) | (EmojiLook::Failed, EmojiLook::Failed) => {
                true
            }
            (
                EmojiLook::Ready { pixels, aspect },
                EmojiLook::Ready {
                    pixels: other,
                    aspect: other_aspect,
                },
            ) => pixels.signature() == other.signature() && aspect == other_aspect,
            _ => false,
        }
    }
}

// An aspect is a finite ratio of two sides, never NaN, so equality is total.
impl Eq for EmojiLook {}

impl std::hash::Hash for EmojiLook {
    /// Hashes as [`PartialEq`] compares, so a look can be part of a row's `memo` version.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        if let EmojiLook::Ready { pixels, aspect } = self {
            pixels.signature().hash(state);
            aspect.to_bits().hash(state);
        }
    }
}

/// A fetch to run: emoji `key`'s image from `source`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EmojiJob {
    pub(crate) key: EmojiKey,
    pub(crate) source: MediaSource,
}

struct Entry {
    /// The image's revision (the emoji's creation time), so a re-created emoji is fetched anew.
    revision: String,
    /// The last tick a view wanted it.
    used: u64,
    /// Its fetch failed (offline, the server): asked again when the account reconnects. A
    /// failed decode is not.
    fetch_failed: bool,
}

/// Why an emoji's image is not here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EmojiFailure {
    /// Its bytes could not be read: worth asking again once connected.
    Fetch,
    /// Its bytes are not an image Papo decodes: final.
    Decode,
}

/// What every emoji draws as now: a cheap snapshot views read.
#[derive(Clone, Default)]
pub(crate) struct EmojiLooks(Arc<HashMap<EmojiKey, EmojiLook>>);

impl EmojiLooks {
    /// What emoji `key` draws as; loading until asked and landed.
    pub(crate) fn look(&self, key: EmojiKey) -> EmojiLook {
        self.0.get(&key).cloned().unwrap_or(EmojiLook::Loading)
    }

    /// What the custom emoji of `account` that `content` names draw as, in order: part of a
    /// memoized row's version, so the row redraws as their images land.
    pub(crate) fn of_content(
        &self,
        account: AccountId,
        content: &papo_sdk_ui::message_content::MessageContent,
    ) -> Vec<EmojiLook> {
        crate::timeline::content::custom_emoji_ids(content)
            .into_iter()
            .map(|id| self.look(EmojiKey { account, id }))
            .collect()
    }

    /// What account `account`'s emoji at `emoji` (ranges of a text) draw as.
    pub(crate) fn of(
        &self,
        account: AccountId,
        emoji: &[(std::ops::Range<usize>, Uuid)],
    ) -> Vec<(std::ops::Range<usize>, EmojiLook)> {
        emoji
            .iter()
            .map(|(range, id)| (range.clone(), self.look(EmojiKey { account, id: *id })))
            .collect()
    }
}

/// The decoded emoji images.
#[derive(Default)]
pub(crate) struct EmojiImages {
    entries: HashMap<EmojiKey, Entry>,
    looks: EmojiLooks,
    tick: u64,
}

impl EmojiImages {
    /// What every emoji draws as now.
    pub(crate) fn looks(&self) -> EmojiLooks {
        self.looks.clone()
    }

    /// The views want `emojis` of `account` now. Returns the fetches to start: one for each
    /// emoji not yet asked for, or asked for at an older revision.
    pub(crate) fn want<'a>(
        &mut self,
        account: AccountId,
        emojis: impl IntoIterator<Item = &'a CustomEmoji>,
    ) -> Vec<EmojiJob> {
        self.tick = self.tick.wrapping_add(1);
        let tick = self.tick;
        let mut jobs = Vec::new();
        for emoji in emojis {
            let key = EmojiKey {
                account,
                id: emoji.id,
            };
            match self.entries.get_mut(&key) {
                Some(entry) if entry.revision == emoji.created_at => entry.used = tick,
                _ => {
                    self.entries.insert(
                        key,
                        Entry {
                            revision: emoji.created_at.clone(),
                            used: tick,
                            fetch_failed: false,
                        },
                    );
                    Arc::make_mut(&mut self.looks.0).remove(&key);
                    jobs.push(EmojiJob {
                        key,
                        source: emoji.source(),
                    });
                }
            }
        }
        self.evict();
        jobs
    }

    /// Emoji `ids` of `account` that a view names but its catalog lacks (it changed since the
    /// content was parsed): they show their name (§9). Returns whether a view draws something
    /// else now.
    pub(crate) fn unknown(
        &mut self,
        account: AccountId,
        ids: impl IntoIterator<Item = Uuid>,
    ) -> bool {
        let looks = Arc::make_mut(&mut self.looks.0);
        let mut changed = false;
        for id in ids {
            let key = EmojiKey { account, id };
            if self.entries.remove(&key).is_some() || looks.get(&key) != Some(&EmojiLook::Failed) {
                looks.insert(key, EmojiLook::Failed);
                changed = true;
            }
        }
        changed
    }

    /// Account `account` is connected again: emoji whose fetch failed are asked for anew at
    /// the next want, loading meanwhile. Returns whether a view draws something else now.
    pub(crate) fn reconnected(&mut self, account: AccountId) -> bool {
        let retried: Vec<EmojiKey> = self
            .entries
            .iter()
            .filter(|(key, entry)| key.account == account && entry.fetch_failed)
            .map(|(key, _)| *key)
            .collect();
        let looks = Arc::make_mut(&mut self.looks.0);
        for key in &retried {
            self.entries.remove(key);
            looks.remove(key);
        }
        !retried.is_empty()
    }

    /// Job `job` ended with `image` (its pixels and their aspect), or why it has none.
    /// Returns whether a view draws something else now.
    pub(crate) fn landed(
        &mut self,
        job: &EmojiJob,
        image: Result<(Pixels, f32), EmojiFailure>,
    ) -> bool {
        let MediaSource::Emoji { revision, .. } = &job.source else {
            return false;
        };
        let Some(entry) = self.entries.get_mut(&job.key) else {
            // Evicted while it was fetched.
            return false;
        };
        if entry.revision != *revision {
            return false;
        }
        entry.fetch_failed = matches!(image, Err(EmojiFailure::Fetch));
        let look = match image {
            Ok((pixels, aspect)) => EmojiLook::Ready { pixels, aspect },
            Err(_) => EmojiLook::Failed,
        };
        Arc::make_mut(&mut self.looks.0).insert(job.key, look);
        true
    }

    /// Account `account` is gone: its images go too.
    pub(crate) fn forget(&mut self, account: AccountId) {
        self.entries.retain(|key, _| key.account != account);
        Arc::make_mut(&mut self.looks.0).retain(|key, _| key.account != account);
    }

    /// Keeps the [`MAX_KEPT`] most recently wanted, never one still loading nor one the views
    /// want now (more than that shown at once keeps them all rather than flicker).
    fn evict(&mut self) {
        let over = self.entries.len().saturating_sub(MAX_KEPT);
        if over == 0 {
            return;
        }
        let tick = self.tick;
        let mut settled: Vec<(u64, EmojiKey)> = self
            .entries
            .iter()
            .filter(|(key, entry)| entry.used != tick && self.looks.0.contains_key(key))
            .map(|(key, entry)| (entry.used, *key))
            .collect();
        settled.sort_unstable_by_key(|(used, _)| *used);
        let looks = Arc::make_mut(&mut self.looks.0);
        for (_, key) in settled.into_iter().take(over) {
            self.entries.remove(&key);
            looks.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests;
