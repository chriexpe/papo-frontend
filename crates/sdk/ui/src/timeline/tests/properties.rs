//! Fixed-seed snapshot/diff equivalence for actual shared-source window state and bounded publishers.

use proptest::{
    prelude::*,
    test_runner::{Config, RngAlgorithm, TestRng, TestRunner},
};

use super::*;

fn paginate(fixture: &mut Fixture, action: u8, selector: u8) {
    match action {
        3 | 4 => {
            let id = fixture
                .source
                .messages
                .first_key_value()
                .map_or(1, |(id, _)| id.saturating_sub(1));
            fixture
                .source
                .messages
                .entry(id)
                .or_insert_with(|| message(id));
            fixture.publish(1);
            let cursor = fixture.source.messages[&id].cursor().unwrap();
            fixture.extend(usize::from(action == 4), TimelineDirection::Older, &cursor);
        }
        5 | 6 => {
            let index = usize::from(action == 5);
            let message = fixture
                .source
                .messages
                .values()
                .rev()
                .nth(usize::from(selector % 3))
                .unwrap();
            let cursor = message.cursor().unwrap();
            fixture.extend(index, TimelineDirection::Newer, &cursor);
        }
        _ => unreachable!("pagination test action"),
    }
}

fn step(fixture: &mut Fixture, action: u8, selector: u8) -> bool {
    let bounds = fixture
        .views
        .each_ref()
        .map(|view| (view.state.bounds.lower, view.state.bounds.upper));
    match action {
        0 => {
            fixture.source.arrival();
            fixture.publish(1);
        }
        1 => {
            fixture.source.edit(13 + u128::from(selector % 8));
            fixture.publish(1);
        }
        2 => {
            fixture
                .source
                .messages
                .remove(&(13 + u128::from(selector % 8)));
            fixture.publish(1);
        }
        3..=6 => paginate(fixture, action, selector),
        7 => {
            let mut reading = (**fixture.reading.borrow()).clone();
            reading.version += 1;
            reading.generation = Generation::for_test(fixture.source.generation);
            reading.channels.insert(
                CHANNEL,
                ReadState {
                    first_unread: Some(Uuid::from_u128((13 + u128::from(selector % 8)) * 100)),
                    ..ReadState::default()
                },
            );
            fixture.reading.send_replace(Arc::new(reading));
            fixture.refresh();
        }
        8 => {
            let view = &mut fixture.views[usize::from(selector % 2)];
            let direction = usize::from((selector / 2) % 2);
            view.state.loading[direction] = !view.state.loading[direction];
            view.state.reproject(&fixture.config, false).unwrap();
            view.record();
        }
        9 | 10 => {
            let resets = fixture.views.each_ref().map(|view| view.fast.resets);
            fixture.source.generation += 1;
            fixture.source.gap = action == 10;
            if action == 10 {
                fixture.source.arrival();
            }
            fixture.publish(1);
            for (view, before) in fixture.views.iter().zip(resets) {
                assert_eq!(
                    view.fast.resets,
                    before + 1,
                    "reconnect must publish a coherent reset"
                );
            }
        }
        11 => {
            let resets = fixture.views.each_ref().map(|view| view.fast.resets);
            fixture.source.arrival();
            fixture.publish(3);
            for (view, before) in fixture.views.iter().zip(resets) {
                assert_eq!(
                    view.fast.resets,
                    before + 1,
                    "coalesced raw versions must reset"
                );
            }
        }
        12 => {
            for view in &mut fixture.views {
                view.checkpoint();
            }
        }
        _ => return false,
    }
    assert!(Arc::ptr_eq(
        &fixture.views[0].state.raw,
        &fixture.views[1].state.raw
    ));
    if action != 3 {
        assert_eq!(
            fixture.views[0].state.bounds.lower, bounds[0].0,
            "only that live window's older page may extend its lower bound"
        );
    }
    if action != 4 {
        assert_eq!(
            fixture.views[1].state.bounds.lower, bounds[1].0,
            "another window cannot extend focused older bounds"
        );
    }
    if action != 5 {
        assert_eq!(
            fixture.views[1].state.bounds.upper, bounds[1].1,
            "live arrivals and another window's pages cannot extend focused newer bounds"
        );
    }
    true
}

#[test]
fn interleaved_windows_and_fast_or_stalled_subscribers_reconstruct_every_published_snapshot() {
    let actions = prop::collection::vec((0u8..14, any::<u8>()), 1..90);
    let mut runner = TestRunner::new_with_rng(
        Config {
            cases: 128,
            failure_persistence: None,
            ..Config::default()
        },
        TestRng::deterministic_rng(RngAlgorithm::ChaCha),
    );
    runner
        .run(&actions, |actions| {
            let mut fixture = Fixture::new();
            // Every case covers every transition family before randomized ordering and checkpoints.
            let required = [
                (0, 0),
                (1, 5),
                (2, 0),
                (3, 0),
                (4, 0),
                (5, 0),
                (6, 0),
                (7, 1),
                (8, 0),
                (8, 3),
                (9, 0),
                (10, 0),
                (11, 0),
                (12, 0),
            ];
            for (action, selector) in required.into_iter().chain(actions) {
                if !step(&mut fixture, action, selector) {
                    break;
                }
            }
            for view in &mut fixture.views {
                view.checkpoint();
                prop_assert!(
                    view.stalled.resets > 0,
                    "stalled queue must exercise overflow Reset"
                );
            }
            fixture.close();
            Ok(())
        })
        .unwrap();
}

#[test]
fn reconnect_overlap_removes_old_gap_items_without_replacing_unchanged_message_arcs() {
    let mut fixture = Fixture::new();
    fixture.extend(1, TimelineDirection::Older, &message(10).cursor().unwrap());
    fixture.extend(1, TimelineDirection::Newer, &message(20).cursor().unwrap());
    step(&mut fixture, 10, 0);
    assert!(
        fixture.views[1]
            .state
            .publisher
            .items
            .iter()
            .any(|item| matches!(item.as_ref(), TimelineItem::Gap(_)))
    );
    let before = message_items(&fixture.views[1].state.publisher.items);
    step(&mut fixture, 9, 0);
    let after = message_items(&fixture.views[1].state.publisher.items);
    assert!(
        !fixture.views[1]
            .state
            .publisher
            .items
            .iter()
            .any(|item| matches!(item.as_ref(), TimelineItem::Gap(_)))
    );
    for (id, item) in after {
        assert!(Arc::ptr_eq(&before[&id], &item));
    }
    for view in &mut fixture.views {
        view.checkpoint();
    }
    fixture.close();
}
