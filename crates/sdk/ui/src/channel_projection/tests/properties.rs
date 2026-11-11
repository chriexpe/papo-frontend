//! Fixed-seed property checks for channel snapshot/diff equivalence and malformed identities.

use proptest::prelude::*;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};

use super::*;

fn runner() -> TestRunner {
    TestRunner::new_with_rng(
        Config {
            cases: 256,
            failure_persistence: None,
            ..Config::default()
        },
        TestRng::deterministic_rng(RngAlgorithm::ChaCha),
    )
}

fn expected(channels: &[Channel]) -> Vector<Arc<ChannelListItem>> {
    channels
        .iter()
        .map(|channel| {
            Arc::new(ChannelListItem {
                id: channel.id.clone(),
                name: channel.name.clone(),
                kind: channel.r#type.clone(),
                position: channel.position,
                topic: channel.topic.clone(),
                created_at: channel.created_at.clone(),
                unread: None,
            })
        })
        .collect()
}

#[test]
fn every_delivered_batch_reconstructs_the_next_snapshot() {
    let actions = prop::collection::vec((0u8..7, 0u8..8, any::<u8>()), 1..80);
    runner()
        .run(&actions, |actions| {
            let mut channels = Vec::new();
            let (sender, receiver) = watch::channel(source(0, 1, vec![]));
            let mut subscription = subscribe_source(receiver);
            let mut version = 0;
            let mut generation = 1;
            for (action, id, revision) in actions {
                let previous = subscription.snapshot.clone();
                let mut publications = 1;
                match action {
                    0 => {
                        channels.retain(|item: &Channel| item.id != Some(id.to_string()));
                        channels.insert(0, channel(id, revision));
                    }
                    1 => {
                        if let Some(item) = channels
                            .iter_mut()
                            .find(|item| item.id == Some(id.to_string()))
                        {
                            item.name = Some(revision.to_string());
                            item.topic = Some(revision.to_string());
                            item.r#type = Some(revision.to_string());
                            item.position = Some(i64::from(revision));
                            item.created_at = Some(revision.to_string());
                        }
                    }
                    2 => channels.retain(|item| item.id != Some(id.to_string())),
                    3 => channels.reverse(),
                    4 => generation += 1,
                    5 => publications = 3,
                    _ => channels = (0..id).rev().map(|key| channel(key, revision)).collect(),
                }
                for _ in 0..publications {
                    version += 1;
                    sender.send_replace(source(version, generation, channels.clone()));
                }
                let update = subscription.stream.next().now_or_never().unwrap().unwrap();
                prop_assert_eq!((update.version, update.generation), (version, generation));
                let reset = matches!(update.diffs.as_slice(), [VectorDiff::Reset { .. }]);
                prop_assert_eq!(reset, publications > 1 || action == 4);
                for diff in update.diffs {
                    diff.apply(&mut subscription.snapshot);
                }
                prop_assert_eq!(&subscription.snapshot, &expected(&channels));
                for item in &subscription.snapshot {
                    if let Some(old) = previous.iter().find(|old| old.as_ref() == item.as_ref()) {
                        prop_assert!(Arc::ptr_eq(old, item));
                    }
                }
                prop_assert!(subscription.stream.next().now_or_never().is_none());
            }
            Ok(())
        })
        .unwrap();
}

#[test]
fn arbitrary_optional_and_duplicate_ids_never_lose_backend_rows() {
    let records = prop::collection::vec(
        (
            prop::option::of(0u8..4),
            any::<u8>(),
            prop::option::of(-5i64..5),
        ),
        0..12,
    );
    let transitions = prop::collection::vec(records, 1..30);
    runner()
        .run(&transitions, |transitions| {
            let (sender, receiver) = watch::channel(source(0, 1, vec![]));
            let mut subscription = subscribe_source(receiver);
            for (version, records) in (1..).zip(transitions) {
                let channels: Vec<_> = records
                    .into_iter()
                    .map(|(id, revision, position)| Channel {
                        id: id.map(|id| id.to_string()),
                        name: Some(revision.to_string()),
                        position,
                        ..Channel::default()
                    })
                    .collect();
                sender.send_replace(source(version, 1, channels.clone()));
                let update = subscription.stream.next().now_or_never().unwrap().unwrap();
                for diff in update.diffs {
                    diff.apply(&mut subscription.snapshot);
                }
                prop_assert_eq!(&subscription.snapshot, &expected(&channels));
            }
            Ok(())
        })
        .unwrap();
}
