//! Unified account-scoped direct-conversation metadata; mutations, notification policy and navigation live elsewhere.

use chrono::DateTime;
use eyeball_im::VectorDiff;
use futures_util::{
    Stream, StreamExt,
    stream::{self, SelectAll},
};
use imbl::Vector;
use papo_sdk::{AccountId, DirectRecord, Generation, ResourceSnapshot, Session, Uuid};
use std::{
    collections::{BTreeMap, BTreeSet},
    pin::Pin,
    sync::Arc,
};
use tokio::sync::watch;

/// Stable conversation identity includes account, even when servers reuse the same remote UUID.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct DirectMessageId {
    /// Originating signed-in account; clients use it for navigation and requests.
    pub account: AccountId,
    /// Remote channel identity, never interpreted as a chronology key.
    pub channel: Uuid,
}
/// Direct-conversation row, without fetching media, profiles or message history.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectMessageItem {
    /// Account-scoped identity.
    pub id: DirectMessageId,
    /// Resource connection generation, validated again before acting.
    pub generation: Generation,
    /// Backend participant identity.
    pub user: Uuid,
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
    /// Capacity-one sources coalesce; edits reconcile directly against the last observed images.
    pub stream: Pin<Box<dyn Stream<Item = DirectMessageUpdate> + Send>>,
}
/// One unified list publication. Account revocation removes only that account's rows.
pub struct DirectMessageUpdate {
    /// Projection version, incremented once for each visible edit batch or generation reset.
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
        subscribe_accounts(
            self.sessions
                .iter()
                .filter(|session| !session.is_closed())
                .map(|session| (session.account(), session.subscribe_resources())),
        )
    }
}

fn subscribe_accounts(
    accounts: impl Iterator<Item = (AccountId, watch::Receiver<Arc<ResourceSnapshot>>)>,
) -> DirectMessageSubscription {
    let mut sources = SelectAll::<Source>::new();
    let mut images = BTreeMap::new();
    for (account, mut receiver) in accounts {
        if images.contains_key(&account) {
            continue;
        }
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
    let items = project(&images, &Vector::new(), |_, _| true);
    let snapshot = items.clone();
    let changes = stream::unfold(
        (sources, images, items, 0_u64),
        |(mut sources, mut images, previous, version)| async move {
            loop {
                let (account, image) = sources.next().await?;
                let old = images.get(&account).cloned();
                let reset = image.as_ref().is_some_and(|next| {
                    old.as_ref()
                        .is_none_or(|old| old.generation != next.generation)
                });
                let mut dirty = BTreeSet::new();
                if let (Some(old), Some(next)) = (old.as_ref(), image.as_ref()) {
                    dirty.extend(next.dms.diff_keys(&old.dms));
                    let users: BTreeSet<_> = next.users.diff_keys(&old.users).collect();
                    let blocks_changed = !next.blocks.same_order(&old.blocks);
                    if blocks_changed || !users.is_empty() {
                        dirty.extend(
                            next.dms
                                .iter()
                                .filter(|dm| {
                                    users.contains(&dm.peer_id)
                                        || (blocks_changed
                                            && next.blocks.contains(dm.peer_id)
                                                != old.blocks.contains(dm.peer_id))
                                })
                                .map(|dm| dm.id),
                        );
                    }
                }
                let changed = reset || image.is_none() || !dirty.is_empty();
                if let Some(image) = image {
                    images.insert(account, image);
                } else {
                    images.remove(&account);
                }
                if !changed {
                    continue;
                }
                let items = project(&images, &previous, |row_account, dm| {
                    row_account == account && (reset || dirty.contains(&dm.id))
                });
                let diffs = if reset {
                    vec![VectorDiff::Reset {
                        values: items.clone(),
                    }]
                } else {
                    crate::list_diff::between(&previous, &items, |item| item.id.clone())
                };
                if diffs.is_empty() {
                    continue;
                }
                let version = version.checked_add(1)?;
                return Some((
                    DirectMessageUpdate { version, diffs },
                    (sources, images, items, version),
                ));
            }
        },
    );
    DirectMessageSubscription {
        version: 0,
        snapshot,
        stream: Box::pin(changes.fuse()),
    }
}
type Source = Pin<Box<dyn Stream<Item = (AccountId, Option<Arc<ResourceSnapshot>>)> + Send>>;

fn project(
    images: &BTreeMap<AccountId, Arc<ResourceSnapshot>>,
    previous: &Vector<Arc<DirectMessageItem>>,
    changed: impl Fn(AccountId, &DirectRecord) -> bool,
) -> Vector<Arc<DirectMessageItem>> {
    let existing: BTreeMap<_, _> = previous.iter().map(|item| (&item.id, item)).collect();
    let mut rows: Vec<_> = images
        .iter()
        .flat_map(|(account, image)| {
            let changed = &changed;
            let existing = &existing;
            image.dms.iter().filter_map(move |dm| {
                if image.blocks.contains(dm.peer_id) {
                    return None;
                }
                let id = DirectMessageId {
                    account: *account,
                    channel: dm.id,
                };
                let previous = existing.get(&id).copied();
                if let Some(previous) = previous
                    && !changed(*account, dm)
                {
                    return Some(Arc::clone(previous));
                }
                let peer = image.users.get(dm.peer_id);
                let item = DirectMessageItem {
                    id,
                    generation: image.generation,
                    user: dm.peer_id,
                    username: peer.and_then(|user| user.username.clone()),
                    nickname: peer.and_then(|user| user.nickname.clone()),
                    activity_at: dm.last_message.as_ref().map_or_else(
                        || dm.created_at.clone(),
                        |message| message.created_at.clone(),
                    ),
                };
                Some(
                    if let Some(old) = previous
                        && old.as_ref() == &item
                    {
                        Arc::clone(old)
                    } else {
                        Arc::new(item)
                    },
                )
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
    rows.into_iter().collect()
}

#[cfg(test)]
mod tests;
