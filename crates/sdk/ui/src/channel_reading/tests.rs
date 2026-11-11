//! Reading-watch overlays preserve channel identities and reject cross-generation badges.

use super::*;
use futures_util::FutureExt;
use papo_sdk::{Generation, ReadState, UnreadCount, Uuid};
use std::collections::BTreeMap;

fn channel(id: u128) -> Arc<ChannelListItem> {
    Arc::new(ChannelListItem {
        id: Some(Uuid::from_u128(id).to_string()),
        name: None,
        kind: None,
        position: None,
        topic: None,
        created_at: None,
        unread: None,
    })
}
fn reading(version: u64, generation: Generation, count: u64) -> Arc<ReadingSnapshot> {
    Arc::new(ReadingSnapshot {
        version,
        generation,
        channels: BTreeMap::from([(
            Uuid::from_u128(1),
            ReadState {
                unread_count: UnreadCount::Exact(count),
                ..ReadState::default()
            },
        )]),
    })
}
fn metadata(generation: Generation) -> ChannelListSubscription {
    ChannelListSubscription {
        version: 1,
        generation,
        snapshot: [channel(1), channel(2)].into_iter().collect(),
        stream: Box::pin(stream::pending()),
    }
}

#[test]
fn read_update_sets_one_channel_and_keeps_unaffected_arc() {
    let generation = Generation::for_test(7);
    let (sender, receiver) = watch::channel(reading(1, generation, 3));
    let mut subscription = subscribe(metadata(generation), receiver);
    let unaffected = Arc::clone(&subscription.snapshot[1]);
    sender.send_replace(reading(2, generation, 0));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert!(matches!(
        update.diffs.as_slice(),
        [eyeball_im::VectorDiff::Set { index: 0, .. }]
    ));
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert!(Arc::ptr_eq(&unaffected, &subscription.snapshot[1]));
    assert_eq!(
        subscription.snapshot[0]
            .unread
            .as_ref()
            .unwrap()
            .unread_count,
        UnreadCount::Exact(0)
    );
}

#[test]
fn read_change_between_capture_and_first_poll_is_delivered_once() {
    let generation = Generation::for_test(1);
    let (sender, receiver) = watch::channel(reading(0, generation, 4));
    let mut subscription = subscribe(metadata(generation), receiver);
    sender.send_replace(reading(1, generation, 2));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert!(update.version > subscription.version);
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert_eq!(
        subscription.snapshot[0]
            .unread
            .as_ref()
            .unwrap()
            .unread_count,
        UnreadCount::Exact(2)
    );
    assert!(subscription.stream.next().now_or_never().is_none());
}

#[test]
fn old_generation_marks_are_not_projected_onto_new_metadata() {
    let (_, receiver) = watch::channel(reading(4, Generation::for_test(1), 12));
    let subscription = subscribe(metadata(Generation::for_test(2)), receiver);
    assert!(
        subscription
            .snapshot
            .iter()
            .all(|item| item.unread.is_none())
    );
}

#[test]
fn coalesced_reading_versions_remain_coherent_without_replacing_other_channels() {
    let generation = Generation::for_test(1);
    let (sender, receiver) = watch::channel(reading(0, generation, 0));
    let mut subscription = subscribe(metadata(generation), receiver);
    let unchanged = Arc::clone(&subscription.snapshot[1]);
    for version in 1..=100 {
        sender.send_replace(reading(version, generation, version));
    }
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert_eq!(
        subscription.snapshot[0]
            .unread
            .as_ref()
            .unwrap()
            .unread_count,
        UnreadCount::Exact(100)
    );
    assert!(Arc::ptr_eq(&unchanged, &subscription.snapshot[1]));
}
