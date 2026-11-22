//! Per-account watch aggregation and read/badge overlays for direct conversations.

use chrono::DateTime;
use eyeball_im::VectorDiff;
use futures_util::{StreamExt, future, stream};
use imbl::Vector;
use papo_sdk::{
    AccountId, BadgesSnapshot, ChannelBadges, ClientHandle, DirectRecord, ReadState,
    ReadingSnapshot, RegistrySnapshot, ResourceSnapshot, Session,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use tokio::sync::watch;

use crate::direct_messages::{
    DirectMessageId, DirectMessageItem, DirectMessageSubscription, DirectMessageUpdate,
};

#[derive(Clone)]
pub(super) struct AccountImages {
    pub resources: Arc<ResourceSnapshot>,
    pub reading: Arc<ReadingSnapshot>,
    pub badges: Arc<BadgesSnapshot>,
}

struct AccountEntry {
    resources: watch::Receiver<Arc<ResourceSnapshot>>,
    reading: watch::Receiver<Arc<ReadingSnapshot>>,
    badges: watch::Receiver<Arc<BadgesSnapshot>>,
    images: AccountImages,
}

impl AccountEntry {
    fn from_watches(
        mut resources: watch::Receiver<Arc<ResourceSnapshot>>,
        mut reading: watch::Receiver<Arc<ReadingSnapshot>>,
        mut badges: watch::Receiver<Arc<BadgesSnapshot>>,
    ) -> Self {
        let images = AccountImages {
            resources: Arc::clone(&resources.borrow_and_update()),
            reading: Arc::clone(&reading.borrow_and_update()),
            badges: Arc::clone(&badges.borrow_and_update()),
        };
        Self {
            resources,
            reading,
            badges,
            images,
        }
    }

    fn closed(&self) -> bool {
        self.resources.has_changed().is_err()
            || self.reading.has_changed().is_err()
            || self.badges.has_changed().is_err()
    }
}

struct ClientTracker {
    handle: ClientHandle,
    registry: watch::Receiver<Arc<RegistrySnapshot>>,
    changes: watch::Receiver<()>,
}

enum PollEvent {
    ClientClosed,
    ClientChanged,
    AccountWatch(AccountId, bool),
}

pub(super) fn subscribe_static(sessions: &[Session]) -> DirectMessageSubscription {
    let mut entries = BTreeMap::new();
    for session in sessions {
        if session.is_closed() || entries.contains_key(&session.account()) {
            continue;
        }
        entries.insert(
            session.account(),
            AccountEntry::from_watches(
                session.subscribe_resources(),
                session.subscribe_reading(),
                session.subscribe_badges(),
            ),
        );
    }
    drive(entries, None)
}

pub(super) fn subscribe_client(handle: ClientHandle) -> DirectMessageSubscription {
    let mut registry = handle.subscribe_registry();
    let mut changes = handle.subscribe_session_changes();
    changes.borrow_and_update();
    let snapshot = Arc::clone(&registry.borrow_and_update());
    let mut entries = BTreeMap::new();
    sync_client_entries(&handle, &snapshot, &mut entries);
    drive(
        entries,
        Some(ClientTracker {
            handle,
            registry,
            changes,
        }),
    )
}

fn sync_client_entries(
    handle: &ClientHandle,
    registry: &RegistrySnapshot,
    entries: &mut BTreeMap<AccountId, AccountEntry>,
) -> BTreeSet<AccountId> {
    let mut active = BTreeSet::new();
    let mut added_or_replaced = BTreeSet::new();
    for saved in &registry.accounts {
        let account = saved.id;
        let Ok(session) = handle.session(account) else {
            continue;
        };
        if session.is_closed() {
            continue;
        }
        active.insert(account);
        let next_resources = session.subscribe_resources();
        let reuse = entries.get(&account).is_some_and(|existing| {
            !existing.closed() && existing.resources.same_channel(&next_resources)
        });
        if !reuse {
            entries.insert(
                account,
                AccountEntry::from_watches(
                    next_resources,
                    session.subscribe_reading(),
                    session.subscribe_badges(),
                ),
            );
            added_or_replaced.insert(account);
        }
    }
    entries.retain(|account, entry| active.contains(account) && !entry.closed());
    added_or_replaced
}

fn drive(
    entries: BTreeMap<AccountId, AccountEntry>,
    tracker: Option<ClientTracker>,
) -> DirectMessageSubscription {
    let snapshots = collect_images(&entries);
    let initial = project(&snapshots, &Vector::new(), |_, _| true);
    let snapshot = initial.clone();
    let stream = stream::unfold(
        (entries, tracker, initial, 0_u64, false),
        |(mut entries, mut tracker, mut previous, mut version, ended)| async move {
            if ended {
                return None;
            }
            loop {
                if tracker.is_none() && entries.is_empty() {
                    return None;
                }
                let event = wait_next(&mut entries, tracker.as_mut()).await;
                let mut reset = false;
                let mut changed_accounts = BTreeSet::new();
                let mut ended_now = false;

                match event {
                    PollEvent::ClientClosed => {
                        ended_now = true;
                        entries.clear();
                    }
                    PollEvent::ClientChanged => {
                        if let Some(client) = &mut tracker {
                            client.changes.borrow_and_update();
                            let reg = Arc::clone(&client.registry.borrow_and_update());
                            changed_accounts.extend(entries.keys().copied());
                            let replaced = sync_client_entries(&client.handle, &reg, &mut entries);
                            changed_accounts.extend(replaced);
                        }
                    }
                    PollEvent::AccountWatch(account, watch_closed) => {
                        if watch_closed {
                            entries.remove(&account);
                            changed_accounts.insert(account);
                        } else if let Some(entry) = entries.get_mut(&account) {
                            let old_gen = entry.images.resources.generation;
                            let next_resources = Arc::clone(&entry.resources.borrow_and_update());
                            let next_reading = Arc::clone(&entry.reading.borrow_and_update());
                            let next_badges = Arc::clone(&entry.badges.borrow_and_update());
                            if next_resources.generation != old_gen {
                                reset = true;
                            }
                            entry.images = AccountImages {
                                resources: next_resources,
                                reading: next_reading,
                                badges: next_badges,
                            };
                            changed_accounts.insert(account);
                        }
                    }
                }

                let snapshots = collect_images(&entries);
                let items = project(&snapshots, &previous, |row_account, _| {
                    reset || changed_accounts.contains(&row_account)
                });
                let diffs = if reset {
                    vec![VectorDiff::Reset {
                        values: items.clone(),
                    }]
                } else {
                    crate::list_diff::between(&previous, &items, |item| item.id.clone())
                };
                previous = items.clone();
                if diffs.is_empty() {
                    if ended_now {
                        return None;
                    }
                    continue;
                }
                version = version.checked_add(1)?;
                return Some((
                    DirectMessageUpdate { version, diffs },
                    (entries, tracker, items, version, ended_now),
                ));
            }
        },
    );
    DirectMessageSubscription {
        version: 0,
        snapshot,
        stream: Box::pin(stream.fuse()),
    }
}

async fn wait_next(
    entries: &mut BTreeMap<AccountId, AccountEntry>,
    mut tracker: Option<&mut ClientTracker>,
) -> PollEvent {
    let client_wait = async {
        match tracker.as_mut() {
            Some(client) => tokio::select! {
                biased;
                res = client.registry.changed() => res.is_err(),
                res = client.changes.changed() => res.is_err(),
            },
            None => future::pending().await,
        }
    };
    let accounts_wait = async {
        if entries.is_empty() {
            future::pending::<(AccountId, bool)>().await
        } else {
            let futs: Vec<_> = entries
                .iter_mut()
                .map(|(&account, entry)| {
                    Box::pin(async move {
                        let closed = tokio::select! {
                            res = entry.resources.changed() => res.is_err(),
                            res = entry.reading.changed() => res.is_err(),
                            res = entry.badges.changed() => res.is_err(),
                        };
                        (account, closed)
                    })
                })
                .collect();
            let (result, _, _) = future::select_all(futs).await;
            result
        }
    };
    tokio::select! {
        biased;
        closed = client_wait => {
            if closed { PollEvent::ClientClosed } else { PollEvent::ClientChanged }
        }
        (account, closed) = accounts_wait => PollEvent::AccountWatch(account, closed),
    }
}

fn collect_images(
    entries: &BTreeMap<AccountId, AccountEntry>,
) -> BTreeMap<AccountId, AccountImages> {
    entries
        .iter()
        .map(|(&account, entry)| (account, entry.images.clone()))
        .collect()
}

pub(super) fn project(
    images: &BTreeMap<AccountId, AccountImages>,
    previous: &Vector<Arc<DirectMessageItem>>,
    changed: impl Fn(AccountId, &DirectRecord) -> bool,
) -> Vector<Arc<DirectMessageItem>> {
    let existing: BTreeMap<_, _> = previous.iter().map(|item| (&item.id, item)).collect();
    let mut rows: Vec<_> = images
        .iter()
        .flat_map(|(account, entry)| {
            let image = &entry.resources;
            let reading = &entry.reading;
            let badges = &entry.badges;
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
                let unread: Option<ReadState> = (reading.generation == image.generation)
                    .then(|| reading.channels.get(&dm.id))
                    .flatten()
                    .cloned();
                let badge: Option<Arc<ChannelBadges>> = (badges.version != 0
                    && badges.generation == image.generation)
                    .then(|| badges.channels.get(&dm.id))
                    .flatten()
                    .cloned();
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
                    unread,
                    badges: badge,
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
