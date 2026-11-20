//! Deterministic channel-watch behavior; no sessions, network, runtime or clock.

use futures_util::{FutureExt, StreamExt};

use super::*;

use eyeball_im::VectorDiff;
use imbl::Vector;
use papo_sdk::{Channel, Generation};

fn uuid(id: u8) -> Uuid {
    Uuid::from_u128(u128::from(id))
}

fn channel(id: u8, revision: u8) -> Channel {
    Channel {
        id: Some(uuid(id).to_string()),
        name: Some(revision.to_string()),
        r#type: Some("text".into()),
        permissions: Some(vec![]),
        ..Channel::default()
    }
}

fn source(version: u64, generation: u64, channels: Vec<Channel>) -> Arc<ResourceSnapshot> {
    let mut image = ResourceSnapshot::fixture(
        Generation::for_test(generation),
        None,
        channels,
        vec![],
        vec![],
        vec![],
        vec![],
        vec![],
        uuid(254),
    )
    .unwrap();
    image.version = version;
    Arc::new(image)
}

#[test]
fn initial_empty_snapshot_preserves_version_and_epoch() {
    let (_sender, receiver) = watch::channel(source(0, 7, vec![]));
    let subscription = subscribe(receiver);
    assert_eq!(
        (subscription.version, subscription.generation),
        (0, Generation::for_test(7))
    );
    assert!(subscription.snapshot.is_empty());
}

#[test]
fn baseline_captures_latest_publication_without_double_apply() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![]));
    sender.send_replace(source(1, 2, vec![channel(1, 0)]));
    let mut subscription = subscribe(receiver);
    assert_eq!(
        (subscription.version, subscription.generation),
        (1, Generation::for_test(2))
    );
    assert_eq!(subscription.snapshot[0].id, uuid(1));
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
    let mut subscription = subscribe(receiver);
    sender.send_replace(source(1, 1, vec![channel(1, 0)]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert_eq!(update.version, 1);
    assert_eq!(subscription.snapshot[0].id, uuid(1));
}

#[test]
fn source_closure_drains_final_unseen_commit_then_ends() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![]));
    let mut subscription = subscribe(receiver);
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
    let mut subscription = subscribe(receiver);
    assert_eq!(subscription.version, 3);
    assert_eq!(subscription.snapshot.len(), 1);
    assert!(subscription.stream.next().now_or_never().unwrap().is_none());
}

#[test]
fn dropping_subscription_releases_the_watch_receiver() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![]));
    drop(subscribe(receiver));
    assert_eq!(sender.receiver_count(), 0);
}

#[test]
fn subscribers_capture_independent_baselines() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![]));
    let mut early = subscribe(receiver.clone());
    sender.send_replace(source(1, 1, vec![channel(1, 0)]));
    let mut later = subscribe(receiver);
    sender.send_replace(source(2, 1, vec![channel(2, 0)]));
    let early = early.stream.next().now_or_never().unwrap().unwrap();
    let later = later.stream.next().now_or_never().unwrap().unwrap();
    assert!(matches!(
        early.diffs.as_slice(),
        [VectorDiff::PushBack { .. }]
    ));
    assert!(
        later
            .diffs
            .iter()
            .all(|diff| !matches!(diff, VectorDiff::Reset { .. }))
    );
}

#[test]
fn coalesced_equal_values_keep_the_baseline_and_emit_no_edits() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![channel(1, 0)]));
    let mut subscription = subscribe(receiver);
    let original = Arc::clone(&subscription.snapshot[0]);
    for version in 1..=1000 {
        sender.send_replace(source(version, 1, vec![channel(1, 0)]));
    }
    assert!(subscription.stream.next().now_or_never().is_none());
    sender.send_replace(source(1001, 1, vec![channel(1, 0), channel(2, 0)]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.version, 1001);
    assert!(matches!(
        update.diffs.as_slice(),
        [VectorDiff::PushBack { .. }]
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
    let mut subscription = subscribe(receiver);
    sender.send_replace(source(9, 2, vec![channel(1, 0)]));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(
        (update.version, update.generation),
        (9, Generation::for_test(2))
    );
    assert!(matches!(
        update.diffs.as_slice(),
        [VectorDiff::Reset { .. }]
    ));
}

#[test]
fn unrelated_final_resource_commit_stays_silent_and_closes_the_stream() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![channel(1, 0)]));
    let mut subscription = subscribe(receiver);
    sender.send_replace(source(1, 1, vec![channel(1, 0)]));
    drop(sender);
    assert!(subscription.stream.next().now_or_never().unwrap().is_none());
}

#[test]
fn backend_permission_and_read_fields_do_not_become_list_policy() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![channel(1, 0)]));
    let mut subscription = subscribe(receiver);
    sender.send_replace(source(
        1,
        1,
        vec![Channel {
            permissions: Some(vec![]),
            last_read_message: Some(uuid(8).to_string()),
            last_read_at: Some("opaque-timestamp".into()),
            ..channel(1, 0)
        }],
    ));
    assert!(subscription.stream.next().now_or_never().is_none());
}

#[test]
fn insertion_reorder_update_and_delete_preserve_unchanged_arcs() {
    let (sender, receiver) = watch::channel(source(0, 1, vec![channel(1, 0), channel(2, 0)]));
    let mut subscription = subscribe(receiver);
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
    let mut subscription = subscribe(receiver);
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
fn missing_empty_or_duplicate_ids_are_inadmissible() {
    for ids in [
        vec![None],
        vec![Some(String::new())],
        vec![Some(uuid(1).to_string()); 2],
    ] {
        let channels = ids
            .into_iter()
            .map(|id| Channel {
                id,
                ..channel(1, 0)
            })
            .collect();
        let result = ResourceSnapshot::fixture(
            Generation::for_test(1),
            None,
            channels,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            uuid(254),
        );
        assert_eq!(result.unwrap_err().group, papo_sdk::CacheResource::Channels);
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
    let subscription = subscribe(receiver);
    let ids: Vec<_> = subscription.snapshot.iter().map(|item| item.id).collect();
    assert_eq!(ids, vec![uuid(3), uuid(2), uuid(1), uuid(4)]);
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
    sender.send_replace(source(1, 1, vec![channel(1, 0)]));
    assert!(notifications.0.load(Ordering::Relaxed) > 0);
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.version, 1);
}

mod properties;
