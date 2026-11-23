//! Aggregate uncertainty, account isolation and stable identity without network or clocks.

use super::{projection::*, *};
use futures_util::{FutureExt, StreamExt};
use papo_sdk::{
    BadgesSnapshot, ChannelBadges, Generation, RegistrySnapshot, SavedAccount, SavedServer,
    ServerAddress, UnixMicros, Uuid, secret::CredentialGeneration,
};
use std::sync::Mutex;
use tokio::sync::watch;

fn registry() -> Arc<RegistrySnapshot> {
    let server = SavedServer {
        address: ServerAddress::parse("https://example.invalid").unwrap(),
        added_at: UnixMicros(0),
    };
    let accounts = [1, 2].map(|user| SavedAccount {
        id: AccountId::new(server.address.id(), Uuid::from_u128(user)),
        username: "fixture".into(),
        display_name: None,
        avatar_ref: None,
        active: user == 1,
        created_at: UnixMicros(0),
        credential_generation: CredentialGeneration::new(1),
    });
    Arc::new(RegistrySnapshot {
        version: 1,
        servers: vec![server],
        accounts: accounts.into(),
        removals: vec![],
    })
}

fn badges(version: u64, generation: Generation, count: u64) -> Arc<BadgesSnapshot> {
    Arc::new(BadgesSnapshot {
        version,
        generation,
        channels_complete: version != 0,
        channels: BTreeMap::from([
            (
                Uuid::from_u128(1),
                Arc::new(ChannelBadges {
                    mention_count: UnreadCount::Exact(count),
                    has_unread: Some(count > 0),
                    direct_message: false,
                }),
            ),
            (
                Uuid::from_u128(2),
                Arc::new(ChannelBadges {
                    mention_count: UnreadCount::AtLeast(2),
                    has_unread: None,
                    direct_message: true,
                }),
            ),
        ]),
    })
}

fn source(
    generation: Generation,
    mut receiver: watch::Receiver<Arc<BadgesSnapshot>>,
) -> (
    Generation,
    watch::Receiver<Arc<BadgesSnapshot>>,
    Arc<BadgesSnapshot>,
) {
    let snapshot = Arc::clone(&receiver.borrow_and_update());
    (generation, receiver, snapshot)
}

#[test]
fn aggregate_preserves_lower_bounds_and_independent_unread_evidence() {
    let generation = Generation::for_test(1);
    let value = aggregate(&badges(1, generation, 3));
    assert_eq!(value.all.mention_count, UnreadCount::AtLeast(5));
    assert_eq!(value.all.has_unread, Some(true));
    assert_eq!(value.dms.mention_count, UnreadCount::AtLeast(2));
    assert_eq!(value.dms.has_unread, None);
    // A server tile beside Direct counts no DM: exact, and no DM doubt.
    assert_eq!(value.channels.mention_count, UnreadCount::Exact(3));
    assert_eq!(value.channels.has_unread, Some(true));
}

#[test]
fn empty_incomplete_channel_census_is_unknown_for_all_channels_and_dms() {
    let snapshot = BadgesSnapshot {
        version: 1,
        generation: Generation::for_test(1),
        channels_complete: false,
        channels: BTreeMap::new(),
    };
    let value = aggregate(&snapshot);
    assert_eq!(value.all, BadgeAggregate::default());
    assert_eq!(value.dms, BadgeAggregate::default());
}

#[test]
fn empty_complete_channel_census_proves_zero_for_all_channels_and_dms() {
    let snapshot = BadgesSnapshot {
        version: 1,
        generation: Generation::for_test(1),
        channels_complete: true,
        channels: BTreeMap::new(),
    };
    let value = aggregate(&snapshot);
    let zero = BadgeAggregate {
        mention_count: UnreadCount::Exact(0),
        has_unread: Some(false),
    };
    assert_eq!(value.all, zero);
    assert_eq!(value.dms, zero);
}

#[test]
fn incomplete_channel_census_retains_proven_positives_without_claiming_exactness() {
    let mut snapshot = (*badges(1, Generation::for_test(1), 3)).clone();
    snapshot.channels_complete = false;
    snapshot.channels.remove(&Uuid::from_u128(2));
    let value = aggregate(&snapshot);
    assert_eq!(value.all.mention_count, UnreadCount::AtLeast(3));
    assert_eq!(value.all.has_unread, Some(true));
    assert_eq!(value.dms, BadgeAggregate::default());
}

#[test]
fn unopened_opened_replaced_and_closed_sessions_never_cross_accounts() {
    let baseline = registry();
    let first = baseline.accounts[0].id;
    let second = baseline.accounts[1].id;
    let server = first.server();
    let generation = Generation::for_test(1);
    let (_registry_tx, registry_rx) = watch::channel(baseline);
    let (changes_tx, changes_rx) = watch::channel(());
    let sessions = Arc::new(Mutex::new(BTreeMap::new()));
    let lookup = Arc::clone(&sessions);
    let mut subscription = subscribe_sources(registry_rx, changes_rx, move |account| {
        lookup
            .lock()
            .unwrap()
            .get(&account)
            .cloned()
            .map(|(generation, receiver)| source(generation, receiver))
    });
    assert_eq!(
        *subscription.snapshot.accounts[&first],
        AccountBadges::default()
    );
    assert_eq!(
        *subscription.snapshot.servers[&server],
        AccountBadges::default()
    );
    let (old_tx, old_rx) = watch::channel(badges(1, generation, 3));
    sessions.lock().unwrap().insert(first, (generation, old_rx));
    changes_tx.send_replace(());
    let opened = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(
        opened.snapshot.accounts[&first].all.mention_count,
        UnreadCount::AtLeast(5)
    );
    assert_eq!(*opened.snapshot.accounts[&second], AccountBadges::default());
    let unaffected = Arc::clone(&opened.snapshot.accounts[&second]);
    let unchanged = Arc::clone(&opened.snapshot.accounts[&first]);
    old_tx.send_replace(badges(2, generation, 3));
    assert!(subscription.stream.next().now_or_never().is_none());
    let replacement_generation = Generation::for_test(2);
    let (_new_tx, new_rx) = watch::channel(badges(1, replacement_generation, 7));
    sessions
        .lock()
        .unwrap()
        .insert(first, (replacement_generation, new_rx));
    changes_tx.send_replace(());
    let replaced = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(
        replaced.snapshot.servers[&server].all.mention_count,
        UnreadCount::AtLeast(9)
    );
    assert!(Arc::ptr_eq(
        &unaffected,
        &replaced.snapshot.accounts[&second]
    ));
    assert!(!Arc::ptr_eq(
        &unchanged,
        &replaced.snapshot.accounts[&first]
    ));
    old_tx.send_replace(badges(3, generation, 100));
    assert!(subscription.stream.next().now_or_never().is_none());
    sessions.lock().unwrap().remove(&first);
    changes_tx.send_replace(());
    let closed = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(*closed.snapshot.accounts[&first], AccountBadges::default());
    assert_eq!(*closed.snapshot.servers[&server], AccountBadges::default());
}

#[test]
fn epoch_change_after_snapshot_capture_keeps_the_new_badge_image_pending() {
    let baseline = registry();
    let account = baseline.accounts[0].id;
    let old_generation = Generation::for_test(1);
    let new_generation = Generation::for_test(2);
    let (_registry_tx, registry_rx) = watch::channel(baseline);
    let (_changes_tx, changes_rx) = watch::channel(());
    let (sender, receiver) = watch::channel(badges(1, old_generation, 3));
    let first_capture = std::sync::atomic::AtomicBool::new(true);
    let mut subscription = subscribe_sources(registry_rx, changes_rx, move |requested| {
        if requested != account {
            return None;
        }
        let mut receiver = receiver.clone();
        let snapshot = Arc::clone(&receiver.borrow_and_update());
        if first_capture.swap(false, std::sync::atomic::Ordering::SeqCst) {
            sender.send_replace(badges(2, new_generation, 9));
        }
        Some((new_generation, receiver, snapshot))
    });
    assert_eq!(
        *subscription.snapshot.accounts[&account],
        AccountBadges::default()
    );
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(
        update.snapshot.accounts[&account].all.mention_count,
        UnreadCount::AtLeast(11)
    );
    assert!(subscription.stream.next().now_or_never().is_none());
}

#[test]
fn initial_or_wrong_generation_badges_are_unknown() {
    let registry = registry();
    let generation = Generation::for_test(1);
    for snapshot in [
        badges(0, generation, 9),
        badges(1, Generation::for_test(2), 9),
    ] {
        let (_sender, receiver) = watch::channel(snapshot);
        let (_, projected) = capture(&registry, &ServerBadgesSnapshot::default(), &|_| {
            Some(source(generation, receiver.clone()))
        });
        assert!(
            projected
                .accounts
                .values()
                .all(|value| **value == AccountBadges::default())
        );
    }
}

#[test]
fn active_selection_uses_only_its_account_and_noop_registry_keeps_arcs() {
    let baseline = registry();
    let first = baseline.accounts[0].id;
    let second = baseline.accounts[1].id;
    let server = first.server();
    let generation = Generation::for_test(1);
    let (registry_tx, registry_rx) = watch::channel(Arc::clone(&baseline));
    let (_changes_tx, changes_rx) = watch::channel(());
    let (_first_tx, first_rx) = watch::channel(badges(1, generation, 3));
    let (_second_tx, second_rx) = watch::channel(badges(1, generation, 8));
    let mut subscription = subscribe_sources(registry_rx, changes_rx, move |account| {
        if account == first {
            Some(source(generation, first_rx.clone()))
        } else if account == second {
            Some(source(generation, second_rx.clone()))
        } else {
            None
        }
    });
    let first_arc = Arc::clone(&subscription.snapshot.accounts[&first]);
    let second_arc = Arc::clone(&subscription.snapshot.accounts[&second]);
    let server_arc = Arc::clone(&subscription.snapshot.servers[&server]);
    assert_eq!(server_arc.all.mention_count, UnreadCount::AtLeast(5));
    registry_tx.send_replace(Arc::clone(&baseline));
    assert!(subscription.stream.next().now_or_never().is_none());
    let mut switched = (*baseline).clone();
    switched.version += 1;
    switched.accounts[0].active = false;
    switched.accounts[1].active = true;
    registry_tx.send_replace(Arc::new(switched));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(
        update.snapshot.servers[&server].all.mention_count,
        UnreadCount::AtLeast(10)
    );
    assert!(Arc::ptr_eq(&first_arc, &update.snapshot.accounts[&first]));
    assert!(Arc::ptr_eq(&second_arc, &update.snapshot.accounts[&second]));
    assert!(!Arc::ptr_eq(&server_arc, &update.snapshot.servers[&server]));
}

#[test]
fn closed_badge_watch_and_registry_shutdown_clear_retained_totals() {
    let baseline = registry();
    let first = baseline.accounts[0].id;
    let generation = Generation::for_test(1);
    let (registry_tx, registry_rx) = watch::channel(baseline);
    let (_changes_tx, changes_rx) = watch::channel(());
    let (badge_tx, badge_rx) = watch::channel(badges(1, generation, 4));
    let mut subscription = subscribe_sources(registry_rx, changes_rx, move |account| {
        (account == first).then(|| source(generation, badge_rx.clone()))
    });
    drop(registry_tx);
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(*update.snapshot.accounts[&first], AccountBadges::default());
    assert!(subscription.stream.next().now_or_never().unwrap().is_none());
    let closed_rx = badge_tx.subscribe();
    drop(badge_tx);
    let (_, projected) = capture(&registry(), &ServerBadgesSnapshot::default(), &|_| {
        Some(source(generation, closed_rx.clone()))
    });
    assert!(
        projected
            .accounts
            .values()
            .all(|value| **value == AccountBadges::default())
    );
}

#[test]
fn direct_messages_join_the_named_accounts_and_stay_a_lower_bound_when_one_is_unknown() {
    let aggregate = |mention_count, has_unread| {
        Arc::new(AccountBadges {
            all: BadgeAggregate::default(),
            channels: BadgeAggregate::default(),
            dms: BadgeAggregate {
                mention_count,
                has_unread,
            },
        })
    };
    let server = ServerAddress::parse("https://one.invalid").unwrap().id();
    let account = |user| AccountId::new(server, Uuid::from_u128(user));
    let mut snapshot = ServerBadgesSnapshot::default();
    assert_eq!(
        snapshot.direct_messages([]),
        BadgeAggregate {
            mention_count: UnreadCount::Exact(0),
            has_unread: Some(false),
        },
        "no account: nothing unread, exactly"
    );
    snapshot
        .accounts
        .insert(account(1), aggregate(UnreadCount::Exact(2), Some(false)));
    snapshot
        .accounts
        .insert(account(2), aggregate(UnreadCount::Exact(3), Some(true)));
    snapshot
        .accounts
        .insert(account(3), aggregate(UnreadCount::Exact(40), Some(true)));
    assert_eq!(
        snapshot.direct_messages(&[account(1), account(2)]),
        BadgeAggregate {
            mention_count: UnreadCount::Exact(5),
            has_unread: Some(true),
        },
        "only the named accounts"
    );
    assert_eq!(
        snapshot.direct_messages(&[account(1), account(9)]),
        BadgeAggregate {
            mention_count: UnreadCount::AtLeast(2),
            has_unread: None,
        },
        "an account without badges is unknown"
    );
}
