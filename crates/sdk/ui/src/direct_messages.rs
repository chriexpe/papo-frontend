//! Unified account-scoped direct-conversation metadata; mutations, notification policy and navigation live elsewhere.

use chrono::DateTime;
use eyeball_im::VectorDiff;
use futures_util::{
    Stream, StreamExt,
    stream::{self, SelectAll},
};
use imbl::Vector;
use papo_sdk::{AccountId, Generation, ResourceSnapshot, Session};
use std::{collections::BTreeMap, pin::Pin, sync::Arc};

/// Stable conversation identity includes account, even when servers reuse the same remote UUID.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DirectMessageId {
    /// Originating signed-in account; clients use it for navigation and requests.
    pub account: AccountId,
    /// Remote channel identity, never interpreted as a chronology key.
    pub channel: String,
}
/// Direct-conversation row, without fetching media, profiles or message history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectMessageItem {
    /// Account-scoped identity.
    pub id: DirectMessageId,
    /// Resource connection generation, validated again before acting.
    pub generation: Generation,
    /// Backend participant identity.
    pub user: String,
    /// Backend username.
    pub username: Option<String>,
    /// Backend nickname.
    pub nickname: Option<String>,
    /// Exact backend last-message timestamp, falling back to conversation creation.
    pub activity_at: String,
}
/// Atomic global baseline followed by sequential list changes.
pub struct DirectMessageSubscription {
    /// Projection version represented by the baseline.
    pub version: u64,
    /// Rows sorted by activity instant descending, then account/channel for deterministic ties.
    pub snapshot: Vector<Arc<DirectMessageItem>>,
    /// Capacity-one sources coalesce; any skipped source version resets the list.
    pub stream: Pin<Box<dyn Stream<Item = DirectMessageUpdate> + Send>>,
}
/// One unified list publication. Account revocation removes only that account's rows.
pub struct DirectMessageUpdate {
    /// Projection version after applying this batch.
    pub version: u64,
    /// Sequential edits, including Reset on a source discontinuity.
    pub diffs: Vec<VectorDiff<Arc<DirectMessageItem>>>,
}
/// Pull-driven aggregation of explicitly supplied signed-in accounts. Recreate when accounts are added.
pub struct DirectMessageService {
    sessions: Vec<Session>,
}
impl DirectMessageService {
    /// Shares sessions and their existing resource watches; never starts another connection.
    #[must_use]
    pub fn new(sessions: Vec<Session>) -> Self {
        Self { sessions }
    }
    /// Captures each independent account atomically with its stream, then merges activity order.
    /// Cached rows convey no permission grant; a later action must use its originating session.
    #[must_use]
    pub fn subscribe(&self) -> DirectMessageSubscription {
        let mut sources = SelectAll::<Source>::new();
        let mut images = BTreeMap::new();
        for session in &self.sessions {
            if session.is_closed() || images.contains_key(&session.account()) {
                continue;
            }
            let account = session.account();
            let mut receiver = session.subscribe_resources();
            images.insert(account, Arc::clone(&receiver.borrow_and_update()));
            let source = stream::unfold(Some(receiver), move |receiver| async move {
                let mut receiver = receiver?;
                if receiver.changed().await.is_err() {
                    return Some(((account, None), None));
                }
                let image = Arc::clone(&receiver.borrow_and_update());
                Some(((account, Some(image)), Some(receiver)))
            });
            sources.push(Box::pin(source));
        }
        let items = project(&images, &Vector::new());
        let snapshot = items.clone();
        let changes = stream::unfold(
            (sources, images, items, 0_u64),
            |(mut sources, mut images, previous, version)| async move {
                let (account, image) = sources.next().await?;
                let reset = image.as_ref().is_some_and(|next| {
                    images.get(&account).is_none_or(|old| {
                        old.generation != next.generation
                            || old.version.checked_add(1) != Some(next.version)
                    })
                });
                let changed = image.as_ref().is_none_or(|next| {
                    images.get(&account).is_none_or(|old| {
                        old.generation != next.generation
                            || !Arc::ptr_eq(&old.dms, &next.dms)
                            || !Arc::ptr_eq(&old.blocks, &next.blocks)
                    })
                });
                if let Some(image) = image {
                    images.insert(account, image);
                } else {
                    images.remove(&account);
                }
                let items = if changed {
                    project(&images, &previous)
                } else {
                    previous.clone()
                };
                let diffs = if reset {
                    vec![VectorDiff::Reset {
                        values: items.clone(),
                    }]
                } else if !changed {
                    Vec::new()
                } else {
                    crate::list_diff::between(&previous, &items, |item| item.id.clone())
                };
                let version = version.checked_add(1)?;
                Some((
                    DirectMessageUpdate { version, diffs },
                    (sources, images, items, version),
                ))
            },
        );
        DirectMessageSubscription {
            version: 0,
            snapshot,
            stream: Box::pin(changes.fuse()),
        }
    }
}
type Source = Pin<Box<dyn Stream<Item = (AccountId, Option<Arc<ResourceSnapshot>>)> + Send>>;

fn project(
    images: &BTreeMap<AccountId, Arc<ResourceSnapshot>>,
    previous: &Vector<Arc<DirectMessageItem>>,
) -> Vector<Arc<DirectMessageItem>> {
    let existing: BTreeMap<_, _> = previous.iter().map(|item| (&item.id, item)).collect();
    let mut rows: Vec<_> = images
        .iter()
        .flat_map(|(account, image)| {
            image.dms.iter().filter_map(move |dm| {
                let user = dm.user.id.clone()?;
                if image
                    .blocks
                    .iter()
                    .any(|blocked| blocked.id.as_deref() == Some(user.as_str()))
                {
                    return None;
                }
                Some(DirectMessageItem {
                    id: DirectMessageId {
                        account: *account,
                        channel: dm.id.clone(),
                    },
                    generation: image.generation,
                    user,
                    username: dm.user.username.clone(),
                    nickname: dm.user.nickname.clone(),
                    activity_at: dm
                        .last_message
                        .as_ref()
                        .and_then(|message| message.created_at.clone())
                        .unwrap_or_else(|| dm.created_at.clone()),
                })
            })
        })
        .collect();
    rows.sort_by_cached_key(|row| {
        (
            std::cmp::Reverse(
                DateTime::parse_from_rfc3339(&row.activity_at)
                    .ok()
                    .map(|time| time.timestamp_micros()),
            ),
            row.id.clone(),
        )
    });
    rows.into_iter()
        .map(|item| {
            if let Some(old) = existing.get(&item.id)
                && old.as_ref() == &item
            {
                return Arc::clone(old);
            }
            Arc::new(item)
        })
        .collect()
}

#[cfg(test)]
mod tests;
