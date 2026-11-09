//! Deterministic projection/watch checks; no SDK storage, network, runtime or clock.

use futures_util::{FutureExt, StreamExt};
use papo_sdk::secret::CredentialGeneration;
use papo_sdk::{AccountId, SavedAccount, SavedServer, ServerAddress, UnixMicros};

use super::*;

fn server() -> SavedServer {
    SavedServer {
        address: ServerAddress::parse("https://example.invalid").unwrap(),
        added_at: UnixMicros(0),
    }
}

fn account(user: u128, active: bool) -> SavedAccount {
    SavedAccount {
        id: AccountId::new(
            server().address.id(),
            format!("00000000-0000-0000-0000-{user:012x}")
                .parse()
                .unwrap(),
        ),
        display_name: None,
        avatar_ref: None,
        active,
        created_at: UnixMicros(0),
        credential_generation: CredentialGeneration::new(1),
    }
}

fn source(version: u64, accounts: Vec<SavedAccount>) -> Arc<RegistrySnapshot> {
    Arc::new(RegistrySnapshot {
        version,
        servers: vec![server()],
        accounts,
        removals: vec![],
    })
}

#[test]
fn subscription_baseline_marks_latest_source_seen() {
    let (sender, receiver) = watch::channel(source(0, vec![]));
    sender.send_replace(source(1, vec![account(1, true)]));
    let mut subscription = subscribe(receiver);
    assert_eq!(subscription.version, 1);
    assert_eq!(subscription.snapshot.accounts.len(), 1);
    assert!(subscription.stream.next().now_or_never().is_none());
}

#[test]
fn commit_after_subscribe_before_first_poll_is_delivered() {
    let (sender, receiver) = watch::channel(source(0, vec![]));
    let mut subscription = subscribe(receiver);
    sender.send_replace(source(1, vec![account(1, true)]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    for diff in update.accounts {
        diff.apply(&mut subscription.snapshot.accounts);
    }
    assert_eq!(
        (update.version, subscription.snapshot.accounts),
        (1, [account(1, true)].into())
    );
}

#[test]
fn active_switch_updates_both_accounts_in_one_batch() {
    let (sender, receiver) = watch::channel(source(0, vec![account(1, true), account(2, false)]));
    let mut subscription = subscribe(receiver);
    sender.send_replace(source(1, vec![account(1, false), account(2, true)]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.accounts.len(), 2);
    for diff in update.accounts {
        diff.apply(&mut subscription.snapshot.accounts);
    }
    assert_eq!(
        subscription.snapshot.active_account(server().address.id()),
        Some(account(2, true).id)
    );
}

#[test]
fn skipped_source_versions_reset_every_list_coherently() {
    let (sender, receiver) = watch::channel(source(0, vec![account(1, true)]));
    let mut subscription = subscribe(receiver);
    for version in 1..=1000 {
        sender.send_replace(source(version, vec![account(2, true)]));
    }
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.version, 1000);
    assert!(matches!(
        update.servers.as_slice(),
        [VectorDiff::Reset { .. }]
    ));
    assert!(matches!(
        update.accounts.as_slice(),
        [VectorDiff::Reset { .. }]
    ));
    assert!(matches!(
        update.removals.as_slice(),
        [VectorDiff::Reset { .. }]
    ));
    for diff in update.servers {
        diff.apply(&mut subscription.snapshot.servers);
    }
    for diff in update.accounts {
        diff.apply(&mut subscription.snapshot.accounts);
    }
    for diff in update.removals {
        diff.apply(&mut subscription.snapshot.removals);
    }
    assert_eq!(
        subscription.snapshot,
        project(&source(1000, vec![account(2, true)]))
    );
    assert!(subscription.stream.next().now_or_never().is_none());
}

#[test]
fn source_close_drains_final_unseen_commit_then_ends() {
    let (sender, receiver) = watch::channel(source(0, vec![]));
    let mut subscription = subscribe(receiver);
    sender.send_replace(source(1, vec![account(1, true)]));
    drop(sender);
    assert_eq!(
        subscription
            .stream
            .next()
            .now_or_never()
            .unwrap()
            .unwrap()
            .version,
        1
    );
    assert!(subscription.stream.next().now_or_never().unwrap().is_none());
}

#[test]
fn source_close_without_changes_ends_immediately() {
    let (sender, receiver) = watch::channel(source(0, vec![]));
    let mut subscription = subscribe(receiver);
    drop(sender);
    assert!(subscription.stream.next().now_or_never().unwrap().is_none());
}

#[test]
fn subscribers_have_independent_version_baselines() {
    let (sender, receiver) = watch::channel(source(0, vec![]));
    let mut early = subscribe(receiver.clone());
    sender.send_replace(source(1, vec![account(1, true)]));
    let mut later = subscribe(receiver);
    sender.send_replace(source(2, vec![account(2, true)]));
    let early = early.stream.next().now_or_never().unwrap().unwrap();
    let later = later.stream.next().now_or_never().unwrap().unwrap();
    assert!(matches!(
        early.accounts.as_slice(),
        [VectorDiff::Reset { .. }]
    ));
    assert!(
        !later
            .accounts
            .iter()
            .any(|diff| matches!(diff, VectorDiff::Reset { .. }))
    );
}

#[test]
fn dropping_subscription_releases_source_receiver() {
    let (sender, receiver) = watch::channel(source(0, vec![]));
    let subscription = subscribe(receiver);
    drop(subscription);
    assert_eq!(sender.receiver_count(), 0);
}

#[test]
fn account_subscription_filters_other_servers_and_keeps_active_flags() {
    let mut foreign = account(2, true);
    foreign.id = AccountId::new(
        ServerAddress::parse("https://example.invalid/other")
            .unwrap()
            .id(),
        foreign.id.user(),
    );
    let (sender, receiver) = watch::channel(source(0, vec![account(1, false), foreign.clone()]));
    let mut subscription = subscribe_accounts(receiver, server().address.id());
    assert_eq!(subscription.snapshot, [account(1, false)].into());
    sender.send_replace(source(1, vec![account(1, true), foreign]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert_eq!(subscription.snapshot, [account(1, true)].into());
}

#[test]
fn account_subscription_resets_on_skipped_unrelated_source_versions() {
    let (sender, receiver) = watch::channel(source(0, vec![account(1, true)]));
    let mut subscription = subscribe_accounts(receiver, server().address.id());
    sender.send_replace(source(1, vec![account(1, true)]));
    sender.send_replace(source(2, vec![account(1, true)]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert!(matches!(
        update.diffs.as_slice(),
        [VectorDiff::Reset { .. }]
    ));
}

#[test]
fn removal_hiding_and_all_list_edits_reconstruct_one_commit() {
    let previous = project(&source(10, vec![account(1, true)]));
    let next = ServerListSnapshot {
        servers: imbl::Vector::new(),
        accounts: imbl::Vector::new(),
        removals: imbl::Vector::new(),
    };
    let batch = update(10, 11, &previous, &next);
    let mut actual = previous;
    for diff in batch.servers {
        diff.apply(&mut actual.servers);
    }
    for diff in batch.accounts {
        diff.apply(&mut actual.accounts);
    }
    for diff in batch.removals {
        diff.apply(&mut actual.removals);
    }
    assert_eq!(actual, next);
}

#[test]
fn pending_pull_observes_later_commit_without_background_work() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Context;

    use futures_util::task::{ArcWake, waker};

    struct Wakes(AtomicUsize);
    impl ArcWake for Wakes {
        fn wake_by_ref(this: &Arc<Self>) {
            this.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    let (sender, receiver) = watch::channel(source(0, vec![]));
    let mut subscription = subscribe(receiver);
    let notifications = Arc::new(Wakes(AtomicUsize::new(0)));
    let waker = waker(Arc::clone(&notifications));
    let mut context = Context::from_waker(&waker);
    assert!(
        subscription
            .stream
            .as_mut()
            .poll_next(&mut context)
            .is_pending()
    );
    sender.send_replace(source(1, vec![account(1, true)]));
    assert!(notifications.0.load(Ordering::Relaxed) > 0);
    let batch = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(batch.version, 1);
}

#[test]
fn unchanged_consecutive_commit_advances_version_without_edits() {
    let (sender, receiver) = watch::channel(source(0, vec![account(1, true)]));
    let mut subscription = subscribe(receiver);
    sender.send_replace(source(1, vec![account(1, true)]));
    let batch = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(batch.version, 1);
    assert!(batch.servers.is_empty() && batch.accounts.is_empty() && batch.removals.is_empty());
}

#[test]
fn clearing_active_selection_keeps_the_account_visible() {
    let (sender, receiver) = watch::channel(source(0, vec![account(1, true)]));
    let mut subscription = subscribe(receiver);
    sender.send_replace(source(1, vec![account(1, false)]));
    let batch = subscription.stream.next().now_or_never().unwrap().unwrap();
    for diff in batch.accounts {
        diff.apply(&mut subscription.snapshot.accounts);
    }
    assert_eq!(
        subscription.snapshot.active_account(server().address.id()),
        None
    );
    assert_eq!(subscription.snapshot.accounts.len(), 1);
}

#[test]
fn version_wrap_cannot_be_mistaken_for_consecutive_commit() {
    let snapshot = project(&source(0, vec![]));
    let batch = update(u64::MAX, 0, &snapshot, &snapshot);
    assert!(matches!(
        batch.servers.as_slice(),
        [VectorDiff::Reset { .. }]
    ));
}
