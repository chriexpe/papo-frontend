//! Pull-driven badge source capture and changed-only aggregation; sessions remain core-owned.

use super::{
    AccountBadges, BadgeAggregate, ServerBadgesSnapshot, ServerBadgesSubscription,
    ServerBadgesUpdate,
};
use futures_util::{future, stream};
use papo_sdk::{
    AccountId, BadgesSnapshot, ChannelBadges, ClientHandle, Generation, RegistrySnapshot,
    UnreadCount,
};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::watch;

type BadgeWatch = watch::Receiver<Arc<BadgesSnapshot>>;
type Source = (Generation, BadgeWatch, Arc<BadgesSnapshot>);

pub(crate) fn subscribe(handle: ClientHandle) -> ServerBadgesSubscription {
    let changes = handle.subscribe_session_changes();
    let registry = handle.subscribe_registry();
    subscribe_sources(registry, changes, move |account| {
        let session = handle.session(account).ok()?;
        let mut source = session.subscribe_badges();
        let snapshot = Arc::clone(&source.borrow_and_update());
        Some((session.generation(), source, snapshot))
    })
}

pub(super) fn subscribe_sources(
    mut registry: watch::Receiver<Arc<RegistrySnapshot>>,
    mut changes: watch::Receiver<()>,
    lookup: impl Fn(AccountId) -> Option<Source> + Send + 'static,
) -> ServerBadgesSubscription {
    changes.borrow_and_update();
    let baseline = Arc::clone(&registry.borrow_and_update());
    let (sources, snapshot) = capture(&baseline, &ServerBadgesSnapshot::default(), &lookup);
    let initial = snapshot.clone();
    let updates = stream::unfold(
        (registry, changes, sources, snapshot, 0_u64, false, lookup),
        |(mut registry, mut changes, mut sources, mut previous, mut version, ended, lookup)| async move {
            if ended {
                return None;
            }
            loop {
                let ended = wait(&mut registry, &mut changes, &mut sources).await;
                changes.borrow_and_update();
                let current = Arc::clone(&registry.borrow_and_update());
                let (next_sources, next) = capture(&current, &previous, &|account| {
                    if ended { None } else { lookup(account) }
                });
                sources = next_sources;
                if next != previous {
                    version = version.checked_add(1)?;
                    let update = ServerBadgesUpdate {
                        version,
                        snapshot: next.clone(),
                    };
                    return Some((
                        update,
                        (registry, changes, sources, next, version, ended, lookup),
                    ));
                }
                previous = next;
                if ended {
                    return None;
                }
            }
        },
    );
    ServerBadgesSubscription {
        version: 0,
        snapshot: initial,
        stream: Box::pin(updates),
    }
}

async fn wait(
    registry: &mut watch::Receiver<Arc<RegistrySnapshot>>,
    changes: &mut watch::Receiver<()>,
    sources: &mut BTreeMap<AccountId, BadgeWatch>,
) -> bool {
    let badges = async {
        if sources.is_empty() {
            future::pending::<()>().await;
        } else {
            let pending: Vec<_> = sources
                .values_mut()
                .map(|source| Box::pin(source.changed()))
                .collect();
            let _ = future::select_all(pending).await;
        }
    };
    tokio::select! {
        biased;
        result = registry.changed() => result.is_err(),
        result = changes.changed() => result.is_err(),
        () = badges => false,
    }
}

pub(super) fn capture(
    registry: &RegistrySnapshot,
    previous: &ServerBadgesSnapshot,
    lookup: &impl Fn(AccountId) -> Option<Source>,
) -> (BTreeMap<AccountId, BadgeWatch>, ServerBadgesSnapshot) {
    let mut sources = BTreeMap::new();
    let mut accounts = BTreeMap::new();
    for account in &registry.accounts {
        let value = lookup(account.id).map_or_else(
            AccountBadges::default,
            |(generation, source, snapshot)| {
                let open = source.has_changed().is_ok();
                let value = if open && snapshot.version != 0 && snapshot.generation == generation {
                    aggregate(&snapshot)
                } else {
                    AccountBadges::default()
                };
                if open {
                    sources.insert(account.id, source);
                }
                value
            },
        );
        accounts.insert(account.id, reuse(previous.accounts.get(&account.id), value));
    }
    let servers = registry
        .servers
        .iter()
        .map(|server| {
            let id = server.address.id();
            let value = registry
                .accounts
                .iter()
                .find(|account| account.id.server() == id && account.active)
                .and_then(|account| accounts.get(&account.id))
                .map_or_else(AccountBadges::default, |value| value.as_ref().clone());
            (id, reuse(previous.servers.get(&id), value))
        })
        .collect();
    (sources, ServerBadgesSnapshot { accounts, servers })
}

fn reuse(previous: Option<&Arc<AccountBadges>>, value: AccountBadges) -> Arc<AccountBadges> {
    previous
        .filter(|old| old.as_ref() == &value)
        .map_or_else(|| Arc::new(value), Arc::clone)
}

pub(super) fn aggregate(snapshot: &BadgesSnapshot) -> AccountBadges {
    AccountBadges {
        all: total(
            snapshot.channels.values().map(AsRef::as_ref),
            snapshot.channels_complete,
        ),
        dms: total(
            snapshot
                .channels
                .values()
                .filter(|channel| channel.direct_message)
                .map(AsRef::as_ref),
            snapshot.channels_complete,
        ),
    }
}

fn total<'a>(
    channels: impl Iterator<Item = &'a ChannelBadges>,
    channels_complete: bool,
) -> BadgeAggregate {
    let mut count = 0_u64;
    let mut exact = channels_complete;
    let mut unread = channels_complete.then_some(false);
    for channel in channels {
        let (lower_bound, complete) = match channel.mention_count {
            UnreadCount::Exact(value) => (value, true),
            UnreadCount::AtLeast(value) => (value, false),
            _ => (0, false),
        };
        let sum = count.checked_add(lower_bound);
        count = sum.unwrap_or(u64::MAX);
        exact &= complete && sum.is_some();
        unread = match (unread, channel.has_unread) {
            (Some(true), _) | (_, Some(true)) => Some(true),
            (Some(false), Some(false)) => Some(false),
            _ => None,
        };
    }
    BadgeAggregate {
        mention_count: if exact {
            UnreadCount::Exact(count)
        } else {
            UnreadCount::AtLeast(count)
        },
        has_unread: unread,
    }
}
