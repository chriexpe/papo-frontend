//! Deterministic channel-watch behavior; no sessions, network, runtime or clock.

use futures_util::{FutureExt, StreamExt};

use super::*;

struct TestSource {
    version: u64,
    generation: u8,
    channels: Vec<Channel>,
}

impl Source for TestSource {
    type Generation = u8;

    fn version(&self) -> u64 {
        self.version
    }

    fn generation(&self) -> u8 {
        self.generation
    }

    fn channels(&self) -> &[Channel] {
        &self.channels
    }
}

fn channel(id: u8, revision: u8) -> Channel {
    Channel {
        id: Some(id.to_string()),
        name: Some(revision.to_string()),
        ..Channel::default()
    }
}

fn source(version: u64, generation: u8, channels: Vec<Channel>) -> Arc<TestSource> {
    Arc::new(TestSource {
        version,
        generation,
        channels,
    })
}

#[test]
fn initial_empty_snapshot_preserves_version_and_epoch() {
    let (_sender, receiver) = watch::channel(source(0, 7, vec![]));
    let subscription = subscribe_source(receiver);
    assert_eq!((subscription.version, subscription.generation), (0, 7));
    assert!(subscription.snapshot.is_empty());
}

#[test]
fn baseline_captures_latest_publication_without_double_apply() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![]));
    sender.send_replace(source(1, 2, vec![channel(1, 0)]));
    let mut subscription = subscribe_source(receiver);
    assert_eq!((subscription.version, subscription.generation), (1, 2));
    assert_eq!(subscription.snapshot[0].id.as_deref(), Some("1"));
    assert!(subscription.stream.next().now_or_never().is_none());
    sender.send_replace(source(2, 2, vec![channel(1, 1)]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert_eq!(update.version, 2);
    assert_eq!(subscription.snapshot[0].name.as_deref(), Some("1"));
    assert!(subscription.stream.next().now_or_never().is_none());
}

#[test]
fn commit_after_capture_before_first_poll_is_not_missed() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![]));
    let mut subscription = subscribe_source(receiver);
    sender.send_replace(source(1, 1, vec![channel(1, 0)]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert_eq!(update.version, 1);
    assert_eq!(subscription.snapshot[0].id.as_deref(), Some("1"));
}

#[test]
fn source_closure_drains_final_unseen_commit_then_ends() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![]));
    let mut subscription = subscribe_source(receiver);
    sender.send_replace(source(1, 1, vec![channel(1, 0)]));
    drop(sender);
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.version, 1);
    assert!(subscription.stream.next().now_or_never().unwrap().is_none());
    assert!(subscription.stream.next().now_or_never().unwrap().is_none());
}

#[test]
fn already_closed_source_returns_baseline_and_ends_immediately() {
    let (sender, receiver) = watch::channel(source(3, 2, vec![channel(1, 0)]));
    drop(sender);
    let mut subscription = subscribe_source(receiver);
    assert_eq!(subscription.version, 3);
    assert_eq!(subscription.snapshot.len(), 1);
    assert!(subscription.stream.next().now_or_never().unwrap().is_none());
}

#[test]
fn dropping_subscription_releases_the_watch_receiver() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![]));
    drop(subscribe_source(receiver));
    assert_eq!(sender.receiver_count(), 0);
}

#[test]
fn subscribers_capture_independent_baselines() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![]));
    let mut early = subscribe_source(receiver.clone());
    sender.send_replace(source(1, 1, vec![channel(1, 0)]));
    let mut later = subscribe_source(receiver);
    sender.send_replace(source(2, 1, vec![channel(2, 0)]));
    let early = early.stream.next().now_or_never().unwrap().unwrap();
    let later = later.stream.next().now_or_never().unwrap().unwrap();
    assert!(matches!(early.diffs.as_slice(), [VectorDiff::Reset { .. }]));
    assert!(
        later
            .diffs
            .iter()
            .all(|diff| !matches!(diff, VectorDiff::Reset { .. }))
    );
}

#[test]
fn overflow_coalesces_to_one_latest_reset_even_when_items_are_unchanged() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![channel(1, 0)]));
    let mut subscription = subscribe_source(receiver);
    let original = Arc::clone(&subscription.snapshot[0]);
    for version in 1..=1000 {
        sender.send_replace(source(version, 1, vec![channel(1, 0)]));
    }
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.version, 1000);
    assert!(matches!(
        update.diffs.as_slice(),
        [VectorDiff::Reset { .. }]
    ));
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert!(Arc::ptr_eq(&original, &subscription.snapshot[0]));
    assert!(subscription.stream.next().now_or_never().is_none());
}

#[test]
fn consecutive_version_with_new_generation_resets_unchanged_items() {
    let (sender, receiver) = watch::channel(source(8, 1, vec![channel(1, 0)]));
    let mut subscription = subscribe_source(receiver);
    sender.send_replace(source(9, 2, vec![channel(1, 0)]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!((update.version, update.generation), (9, 2));
    assert!(matches!(
        update.diffs.as_slice(),
        [VectorDiff::Reset { .. }]
    ));
}

#[test]
fn unrelated_resource_commit_emits_empty_batch_at_new_version() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![channel(1, 0)]));
    let mut subscription = subscribe_source(receiver);
    sender.send_replace(source(1, 1, vec![channel(1, 0)]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.version, 1);
    assert_eq!(update.diffs, vec![]);
}

#[test]
fn backend_permission_and_read_fields_do_not_become_list_policy() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![channel(1, 0)]));
    let mut subscription = subscribe_source(receiver);
    sender.send_replace(source(
        1,
        1,
        vec![Channel {
            permissions: Some(vec![]),
            last_read_message: Some("opaque-id".into()),
            last_read_at: Some("opaque-timestamp".into()),
            ..channel(1, 0)
        }],
    ));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.diffs, vec![]);
}

#[test]
fn insertion_reorder_update_and_delete_preserve_unchanged_arcs() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![channel(1, 0), channel(2, 0)]));
    let mut subscription = subscribe_source(receiver);
    let retained = Arc::clone(&subscription.snapshot[0]);
    sender.send_replace(source(
        1,
        1,
        vec![channel(3, 0), channel(2, 1), channel(1, 0)],
    ));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert!(Arc::ptr_eq(&retained, &subscription.snapshot[2]));
    assert_eq!(subscription.snapshot[1].name.as_deref(), Some("1"));
    sender.send_replace(source(2, 1, vec![channel(1, 0)]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert!(
        update
            .diffs
            .iter()
            .all(|diff| matches!(diff, VectorDiff::Remove { .. }))
    );
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert!(Arc::ptr_eq(&retained, &subscription.snapshot[0]));
}

#[test]
fn metadata_edit_sets_only_the_changed_identity() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![channel(1, 0), channel(2, 0)]));
    let mut subscription = subscribe_source(receiver);
    let old = Arc::clone(&subscription.snapshot[1]);
    sender.send_replace(source(1, 1, vec![channel(1, 0), channel(2, 1)]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert!(matches!(
        update.diffs.as_slice(),
        [VectorDiff::Set { index: 1, .. }]
    ));
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert!(!Arc::ptr_eq(&old, &subscription.snapshot[1]));
}

#[test]
fn missing_empty_or_duplicate_ids_reset_without_dropping_backend_rows() {
    for ids in [vec![None], vec![Some("")], vec![Some("1"), Some("1")]] {
        let channels: Vec<_> = ids
            .into_iter()
            .map(|id| Channel {
                id: id.map(str::to_owned),
                ..Channel::default()
            })
            .collect();
        let expected = project(&channels, &Vector::new());
        let (sender, receiver) = watch::channel(source(0, 1, vec![]));
        let mut subscription = subscribe_source(receiver);
        sender.send_replace(source(1, 1, channels));
        let update = subscription.stream.next().now_or_never().unwrap().unwrap();
        assert!(matches!(
            update.diffs.as_slice(),
            [VectorDiff::Reset { .. }]
        ));
        for diff in update.diffs {
            diff.apply(&mut subscription.snapshot);
        }
        assert_eq!(subscription.snapshot, expected);
        sender.send_replace(source(2, 1, vec![channel(1, 0)]));
        let update = subscription.stream.next().now_or_never().unwrap().unwrap();
        assert!(matches!(
            update.diffs.as_slice(),
            [VectorDiff::Reset { .. }]
        ));
    }
}

#[test]
fn backend_order_is_preserved_for_optional_tied_and_negative_positions() {
    let channels = vec![
        Channel {
            position: Some(9),
            ..channel(3, 0)
        },
        Channel {
            position: None,
            ..channel(2, 0)
        },
        Channel {
            position: Some(-1),
            ..channel(1, 0)
        },
        Channel {
            position: Some(-1),
            ..channel(4, 0)
        },
    ];
    let (_sender, receiver) = watch::channel(source(0, 1, channels));
    let subscription = subscribe_source(receiver);
    let ids: Vec<_> = subscription
        .snapshot
        .iter()
        .map(|item| item.id.as_deref())
        .collect();
    assert_eq!(ids, vec![Some("3"), Some("2"), Some("1"), Some("4")]);
}

#[test]
fn pending_pull_is_woken_by_later_publication() {
    use futures_util::task::{ArcWake, waker};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Context;

    struct Wakes(AtomicUsize);
    impl ArcWake for Wakes {
        fn wake_by_ref(this: &Arc<Self>) {
            this.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let (sender, receiver) = watch::channel(source(0, 1, vec![]));
    let mut subscription = subscribe_source(receiver);
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
    sender.send_replace(source(1, 1, vec![channel(1, 0)]));
    assert!(notifications.0.load(Ordering::Relaxed) > 0);
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.version, 1);
}

mod properties;
