//! Reading-watch overlays preserve channel identities and reject cross-generation badges.

use super::*;
use futures_util::FutureExt;
use papo_sdk::{Generation, ReadState, UnreadCount, Uuid};
use std::collections::BTreeMap;

fn channel(id: u128) -> Arc<ChannelListItem> {
    Arc::new(ChannelListItem {
        id: Uuid::from_u128(id),
        name: None,
        kind: papo_sdk::ChannelKind::Text,
        position: None,
        topic: None,
        parent_id: None,
        notification_settings: None,
        created_at: None,
        unread: None,
        badges: None,
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
fn badges(version: u64, generation: Generation, count: u64) -> Arc<BadgesSnapshot> {
    Arc::new(BadgesSnapshot {
        version,
        generation,
        channels_complete: version != 0,
        channels: BTreeMap::from([(
            Uuid::from_u128(1),
            Arc::new(papo_sdk::ChannelBadges {
                mention_count: UnreadCount::Exact(count),
                has_unread: Some(count > 0),
                direct_message: false,
            }),
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
    let (_badge_sender, badge_receiver) = watch::channel(badges(0, generation, 0));
    let mut subscription = subscribe(metadata(generation), receiver, badge_receiver);
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
    let (_badge_sender, badge_receiver) = watch::channel(badges(0, generation, 0));
    let mut subscription = subscribe(metadata(generation), receiver, badge_receiver);
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
    let (_badge_sender, badge_receiver) = watch::channel(badges(0, Generation::for_test(2), 0));
    let subscription = subscribe(metadata(Generation::for_test(2)), receiver, badge_receiver);
    assert!(
        subscription
            .snapshot
            .iter()
            .all(|item| item.unread.is_none())
    );
}

#[test]
fn badge_changes_preserve_other_items_and_equal_channel_arcs() {
    let generation = Generation::for_test(1);
    let (_reading_sender, reading_receiver) = watch::channel(reading(1, generation, 2));
    let (sender, receiver) = watch::channel(badges(1, generation, 2));
    let mut subscription = subscribe(metadata(generation), reading_receiver, receiver);
    let unaffected = Arc::clone(&subscription.snapshot[1]);
    sender.send_replace(badges(2, generation, 5));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert!(matches!(
        update.diffs.as_slice(),
        [eyeball_im::VectorDiff::Set { index: 0, .. }]
    ));
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert!(Arc::ptr_eq(&unaffected, &subscription.snapshot[1]));
    let retained = Arc::clone(&subscription.snapshot[0]);
    assert_eq!(
        retained.badges.as_ref().unwrap().mention_count,
        UnreadCount::Exact(5)
    );
    let version = update.version;
    sender.send_replace(badges(3, generation, 5));
    assert!(subscription.stream.next().now_or_never().is_none());
    assert!(Arc::ptr_eq(&retained, &subscription.snapshot[0]));
    sender.send_replace(badges(4, generation, 6));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.version, version + 1);
    assert!(matches!(
        update.diffs.as_slice(),
        [eyeball_im::VectorDiff::Set { index: 0, .. }]
    ));
}

#[test]
fn initial_and_cross_generation_badge_images_are_not_projected() {
    let generation = Generation::for_test(2);
    for image in [
        badges(0, generation, 9),
        badges(1, Generation::for_test(1), 9),
    ] {
        let (_reading_sender, reading_receiver) = watch::channel(reading(1, generation, 2));
        let (_badge_sender, badge_receiver) = watch::channel(image);
        let subscription = subscribe(metadata(generation), reading_receiver, badge_receiver);
        assert!(
            subscription
                .snapshot
                .iter()
                .all(|item| item.badges.is_none())
        );
    }
}

#[test]
fn coalesced_reading_versions_remain_coherent_without_replacing_other_channels() {
    let generation = Generation::for_test(1);
    let (sender, receiver) = watch::channel(reading(0, generation, 0));
    let (_badge_sender, badge_receiver) = watch::channel(badges(0, generation, 0));
    let mut subscription = subscribe(metadata(generation), receiver, badge_receiver);
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

#[test]
fn equal_reading_wakeup_does_not_publish_or_advance_version() {
    let generation = Generation::for_test(1);
    let (sender, receiver) = watch::channel(reading(1, generation, 3));
    let (_badge_sender, badge_receiver) = watch::channel(badges(0, generation, 0));
    let mut subscription = subscribe(metadata(generation), receiver, badge_receiver);
    let retained = Arc::clone(&subscription.snapshot[0]);
    sender.send_replace(reading(2, generation, 3));
    assert!(subscription.stream.next().now_or_never().is_none());
    assert!(Arc::ptr_eq(&retained, &subscription.snapshot[0]));
    sender.send_replace(reading(3, generation, 4));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(update.version, subscription.version + 1);
    assert!(matches!(
        update.diffs.as_slice(),
        [eyeball_im::VectorDiff::Set { index: 0, .. }]
    ));
}

#[test]
fn equal_overlay_preserves_generation_transitions_and_explicit_metadata_resets() {
    let generation = Generation::for_test(1);
    let next_generation = Generation::for_test(2);
    let (reading_sender, reading_receiver) = watch::channel(Arc::new(ReadingSnapshot {
        generation,
        version: 0,
        channels: BTreeMap::new(),
    }));
    let (_badge_sender, badge_receiver) = watch::channel(badges(0, generation, 0));
    let mut metadata = metadata(generation);
    metadata.stream = Box::pin(stream::iter([
        ChannelListUpdate {
            version: 2,
            generation: next_generation,
            diffs: vec![],
        },
        ChannelListUpdate {
            version: 3,
            generation: next_generation,
            diffs: vec![eyeball_im::VectorDiff::Reset {
                values: metadata.snapshot.clone(),
            }],
        },
    ]));
    let mut subscription = subscribe(metadata, reading_receiver, badge_receiver);
    for version in 2..=3 {
        let update = subscription.stream.next().now_or_never().unwrap().unwrap();
        assert_eq!(update.version, version);
        assert_eq!(update.generation, next_generation);
        assert!(matches!(
            update.diffs.as_slice(),
            [eyeball_im::VectorDiff::Reset { .. }]
        ));
    }
    drop(reading_sender);
}

thread_local! {
    pub(super) static VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    pub(super) static ROW_CLONES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[test]
fn unrelated_resource_updates_skip_all_500_channels_and_one_read_clones_one_row() {
    let generation = Generation::for_test(1);
    let (read_sender, reading) = watch::channel(self::reading(1, generation, 0));
    let (_badges, badges) = watch::channel(self::badges(0, generation, 0));
    let own = Uuid::from_u128(1_000);
    let channels = (1..=500)
        .map(|id| papo_sdk::Channel {
            id: Some(Uuid::from_u128(id).to_string()),
            r#type: Some("text".into()),
            name: Some(id.to_string()),
            permissions: Some(vec![]),
            ..papo_sdk::Channel::default()
        })
        .collect();
    let user = papo_sdk::UserSummary {
        id: Some(own.to_string()),
        roles: Some(vec![]),
        ..papo_sdk::UserSummary::default()
    };
    let base = papo_sdk::ResourceSnapshot::fixture(
        generation,
        None,
        channels,
        vec![],
        vec![user.clone()],
        vec![],
        vec![],
        vec![],
        own,
    )
    .unwrap();
    let (resources, receiver) = watch::channel(Arc::new(base.clone()));
    let mut subscription = subscribe(
        crate::channel_projection::subscribe(receiver),
        reading,
        badges,
    );
    let mut changed = base
        .with_summary_for_test(
            own,
            own,
            papo_sdk::UserSummary {
                nickname: "changed".into(),
                ..user
            },
        )
        .unwrap();
    changed.version = 1;
    resources.send_replace(Arc::new(changed.clone()));
    VISITS.set(0);
    ROW_CLONES.set(0);
    assert!(subscription.stream.next().now_or_never().is_none());
    assert_eq!(VISITS.get(), 0);
    assert_eq!(ROW_CLONES.get(), 0);

    // A new silent resource image and a pending overlay must drain in one pull.
    changed.version += 1;
    resources.send_replace(Arc::new(changed));
    let before = subscription.snapshot.clone();
    read_sender.send_replace(self::reading(2, generation, 1));
    let update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert_eq!(ROW_CLONES.get(), 1);
    assert!(matches!(
        update.diffs.as_slice(),
        [eyeball_im::VectorDiff::Set { index: 0, .. }]
    ));
    for diff in update.diffs {
        diff.apply(&mut subscription.snapshot);
    }
    assert_eq!(
        before
            .iter()
            .zip(&subscription.snapshot)
            .filter(|(old, new)| Arc::ptr_eq(old, new))
            .count(),
        499
    );

    drop(resources);
    read_sender.send_replace(self::reading(3, generation, 2));
    drop(read_sender);
    let final_update = subscription.stream.next().now_or_never().unwrap().unwrap();
    assert!(matches!(
        final_update.diffs.as_slice(),
        [eyeball_im::VectorDiff::Set { index: 0, value }] if value.unread.as_ref().unwrap().unread_count == UnreadCount::Exact(2)
    ));
    assert!(subscription.stream.next().now_or_never().unwrap().is_none());
}
