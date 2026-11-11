//! Bounded list subscription guarantees without network, runtime, clock or randomness.

use super::*;
use futures_util::{FutureExt, StreamExt};

#[test]
fn baseline_then_front_batch_append_and_edit_reconstruct_every_snapshot() {
    let mut publisher = Publisher::<_, u8, ()>::new(0, 1, [2, 3].into_iter().collect());
    let mut receiver = publisher.subscribe(64);
    let mut actual = publisher.items.clone();
    for (version, next) in [
        (1, vec![1, 2, 3]),
        (2, vec![1, 2, 3, 4]),
        (3, vec![1, 2, 4]),
    ] {
        publisher.publish(version, 1, next.iter().copied().collect(), false, |item| {
            *item
        });
        let Event::Update {
            version: delivered,
            diffs,
            ..
        } = receiver.next().now_or_never().unwrap().unwrap()
        else {
            panic!("closed early");
        };
        assert_eq!(delivered, version);
        if version == 2 {
            assert!(matches!(
                diffs.as_slice(),
                [VectorDiff::PushBack { value: 4 }]
            ));
        }
        for diff in diffs {
            diff.apply(&mut actual);
        }
        assert_eq!(actual, publisher.items);
    }
}

#[test]
fn overflow_discards_queued_diffs_and_resets_to_current_snapshot() {
    let mut publisher = Publisher::<_, u8, ()>::new(0, 1, Vector::<u8>::new());
    let mut receiver = publisher.subscribe(2);
    for version in 1..=4 {
        publisher.publish(
            version,
            1,
            [u8::try_from(version).unwrap()].into_iter().collect(),
            false,
            |item| *item,
        );
    }
    let Event::Update { version, diffs, .. } = receiver.next().now_or_never().unwrap().unwrap()
    else {
        panic!("closed early");
    };
    assert_eq!(version, 4);
    assert!(matches!(diffs.as_slice(), [VectorDiff::Reset { .. }]));
    let mut items = Vector::new();
    for diff in diffs {
        diff.apply(&mut items);
    }
    assert_eq!(items, publisher.items);
    assert!(receiver.next().now_or_never().is_none());
}

#[test]
fn independent_subscribers_overflow_independently_and_close_once() {
    let mut publisher = Publisher::<_, u8, u8>::new(0, 1, Vector::<u8>::new());
    let mut slow = publisher.subscribe(1);
    let mut fast = publisher.subscribe(1);
    for version in 1..=3 {
        publisher.publish(
            version,
            1,
            [u8::try_from(version).unwrap()].into_iter().collect(),
            false,
            |item| *item,
        );
        assert!(matches!(
            fast.next().now_or_never(),
            Some(Some(Event::Update { .. }))
        ));
    }
    assert!(
        matches!(slow.next().now_or_never(), Some(Some(Event::Update { diffs, .. })) if matches!(diffs.as_slice(), [VectorDiff::Reset { .. }]))
    );
    publisher.close(9);
    publisher.close(8);
    for receiver in [&mut slow, &mut fast] {
        assert!(matches!(
            receiver.next().now_or_never(),
            Some(Some(Event::Closed(9)))
        ));
        assert!(matches!(receiver.next().now_or_never(), Some(None)));
    }
    let mut late = publisher.subscribe(1);
    assert!(matches!(
        late.next().now_or_never(),
        Some(Some(Event::Closed(9)))
    ));
}
