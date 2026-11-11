//! Production window/publisher regressions; generated interleavings live beside this fixture.

mod properties;
mod source;

use source::{Source, message};

use std::{collections::BTreeMap, sync::Arc};

use eyeball_im::VectorDiff;
use futures_util::{FutureExt, StreamExt};
use imbl::Vector;
use papo_sdk::{
    Generation, HistoryClosedReason, HistorySubscription, HistoryUpdate, ReadState,
    ReadingSnapshot, Uuid,
};
use tokio::sync::watch;

use super::{TimelineConfig, TimelineError, state::WindowState};
use crate::{
    list_subscription::{Event, Receiver},
    timeline_item::{TimelineDirection, TimelineItem},
};

const CHANNEL: Uuid = Uuid::from_u128(90_000);
type Items = Vector<Arc<TimelineItem>>;
type Images = BTreeMap<u64, (Generation, Items)>;

struct Consumer {
    receiver: Receiver<Arc<TimelineItem>, Generation, HistoryClosedReason>,
    items: Items,
    version: u64,
    generation: Generation,
    resets: usize,
    closed: Option<HistoryClosedReason>,
}

impl Consumer {
    fn new(state: &mut WindowState, capacity: usize) -> Self {
        Self {
            items: state.publisher.items.clone(),
            version: state.publisher.version,
            generation: state.publisher.generation,
            receiver: state.publisher.subscribe(capacity),
            resets: 0,
            closed: None,
        }
    }

    fn drain(&mut self, state: &WindowState, images: &Images) {
        while let Some(Some(event)) = self.receiver.next().now_or_never() {
            match event {
                Event::Update {
                    version,
                    generation,
                    diffs,
                } => {
                    assert!(version > self.version);
                    let reset = matches!(diffs.as_slice(), [VectorDiff::Reset { .. }]);
                    if version != self.version + 1 || generation != self.generation {
                        assert!(reset, "a skipped version or generation must reset");
                    }
                    self.resets += usize::from(reset);
                    for diff in diffs {
                        diff.apply(&mut self.items);
                    }
                    let (expected_generation, expected_items) = &images[&version];
                    assert!(
                        self.items == *expected_items,
                        "batch must reconstruct its exact published version"
                    );
                    assert_eq!(generation, *expected_generation);
                    self.version = version;
                    self.generation = generation;
                }
                Event::Closed(reason) => {
                    assert!(self.closed.is_none());
                    self.closed = Some(reason);
                    self.items.clear();
                }
            }
        }
        assert!(
            self.items == state.publisher.items,
            "drained diffs must equal current publisher snapshot"
        );
        if self.closed.is_none() {
            assert_eq!(self.version, state.publisher.version);
            assert_eq!(self.generation, state.publisher.generation);
        } else {
            assert!(matches!(self.receiver.next().now_or_never(), Some(None)));
        }
    }
}

struct View {
    state: WindowState,
    fast: Consumer,
    stalled: Consumer,
    images: Images,
}

impl View {
    fn new(mut state: WindowState) -> Self {
        let fast = Consumer::new(&mut state, 64);
        let stalled = Consumer::new(&mut state, 2);
        let images = BTreeMap::from([(
            state.publisher.version,
            (state.publisher.generation, state.publisher.items.clone()),
        )]);
        Self {
            state,
            fast,
            stalled,
            images,
        }
    }

    fn record(&mut self) {
        self.images.insert(
            self.state.publisher.version,
            (
                self.state.publisher.generation,
                self.state.publisher.items.clone(),
            ),
        );
        self.fast.drain(&self.state, &self.images);
    }

    fn checkpoint(&mut self) {
        self.stalled.drain(&self.state, &self.images);
    }
}

struct Fixture {
    source: Source,
    raw: watch::Sender<HistoryUpdate>,
    reading: watch::Sender<Arc<ReadingSnapshot>>,
    views: [View; 2],
    config: TimelineConfig,
}

impl Fixture {
    fn new() -> Self {
        let source = Source::new();
        let snapshot = source.snapshot();
        let raw = watch::channel(HistoryUpdate::Reset(Arc::clone(&snapshot))).0;
        let reading = watch::channel(Arc::new(ReadingSnapshot {
            generation: snapshot.generation,
            version: 0,
            channels: BTreeMap::new(),
        }))
        .0;
        let config = TimelineConfig {
            page_size: 4,
            subscriber_capacity: 2,
            ..TimelineConfig::default()
        };
        let open = |first, last, live| {
            let initial = source
                .messages
                .range(first..=last)
                .map(|(_, message)| Arc::clone(message))
                .collect();
            View::new(
                WindowState::new(
                    HistorySubscription {
                        snapshot: Arc::clone(&snapshot),
                        updates: raw.subscribe(),
                    },
                    reading.subscribe(),
                    initial,
                    live,
                    CHANNEL,
                    &config,
                )
                .unwrap(),
            )
        };
        let views = [open(17, 20, true), open(13, 15, false)];
        Self {
            source,
            raw,
            reading,
            views,
            config,
        }
    }

    fn publish(&mut self, count: u64) {
        for _ in 0..count {
            self.source.version += 1;
            self.raw
                .send_replace(HistoryUpdate::Reset(self.source.snapshot()));
        }
        self.refresh();
    }

    fn refresh(&mut self) {
        for view in &mut self.views {
            let before = view.state.publisher.items.clone();
            view.state.refresh(&self.config).unwrap();
            for item in &view.state.publisher.items {
                if let Some(old) = before.iter().find(|old| old.as_ref() == item.as_ref()) {
                    assert!(
                        Arc::ptr_eq(old, item),
                        "unchanged projected items must keep identity"
                    );
                }
                if let TimelineItem::Message { id, message } = item.as_ref() {
                    let canonical = view
                        .state
                        .raw
                        .messages
                        .iter()
                        .find(|candidate| {
                            candidate
                                .id
                                .as_deref()
                                .and_then(|id| id.parse::<Uuid>().ok())
                                == Some(*id)
                        })
                        .unwrap();
                    assert!(Arc::ptr_eq(message, canonical));
                }
            }
            view.record();
        }
    }

    fn extend(
        &mut self,
        index: usize,
        direction: TimelineDirection,
        cursor: &papo_sdk::HistoryCursor,
    ) {
        let other = 1 - index;
        let untouched = (
            self.views[other].state.bounds.lower,
            self.views[other].state.bounds.upper,
        );
        let state = &mut self.views[index].state;
        let before = (state.bounds.lower, state.bounds.upper);
        state.include(cursor).unwrap();
        match direction {
            TimelineDirection::Older => assert_eq!(state.bounds.upper, before.1),
            TimelineDirection::Newer => assert_eq!(state.bounds.lower, before.0),
        }
        state.reproject(&self.config, false).unwrap();
        self.views[index].record();
        assert_eq!(
            untouched,
            (
                self.views[other].state.bounds.lower,
                self.views[other].state.bounds.upper
            )
        );
    }

    fn close(&mut self) {
        self.raw
            .send_replace(HistoryUpdate::Closed(HistoryClosedReason::ChannelRemoved));
        for view in &mut self.views {
            assert!(matches!(
                view.state.refresh(&self.config),
                Err(TimelineError::Closed(HistoryClosedReason::ChannelRemoved))
            ));
            view.fast.drain(&view.state, &view.images);
            view.checkpoint();
            assert_eq!(view.fast.closed, Some(HistoryClosedReason::ChannelRemoved));
            assert_eq!(
                view.stalled.closed,
                Some(HistoryClosedReason::ChannelRemoved)
            );
        }
        self.source.version += 1;
        self.raw
            .send_replace(HistoryUpdate::Reset(self.source.snapshot()));
        for view in &mut self.views {
            assert!(matches!(
                view.state.refresh(&self.config),
                Err(TimelineError::Closed(HistoryClosedReason::ChannelRemoved))
            ));
        }
    }
}

fn message_items(items: &Items) -> BTreeMap<Uuid, Arc<TimelineItem>> {
    items
        .iter()
        .filter_map(|item| match item.as_ref() {
            TimelineItem::Message { id, .. } => Some((*id, Arc::clone(item))),
            _ => None,
        })
        .collect()
}

#[test]
fn one_edit_changes_exactly_one_item_in_each_window_without_replacing_other_arcs() {
    let mut fixture = Fixture::new();
    fixture.extend(1, TimelineDirection::Newer, &message(19).cursor().unwrap());
    let before = fixture
        .views
        .each_ref()
        .map(|view| message_items(&view.state.publisher.items));
    fixture.source.edit(18);
    fixture.publish(1);
    for (view, previous) in fixture.views.iter_mut().zip(before) {
        let current = message_items(&view.state.publisher.items);
        let changed: Vec<_> = current
            .iter()
            .filter(|(id, item)| !Arc::ptr_eq(&previous[id], item))
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(changed, [Uuid::from_u128(1800)]);
        view.checkpoint();
    }
}

#[test]
fn atomic_baselines_and_stalled_overflow_reconstruct_the_current_items() {
    let mut fixture = Fixture::new();
    for _ in 0..7 {
        fixture.source.arrival();
        fixture.publish(1);
    }
    for view in &mut fixture.views {
        view.checkpoint();
        assert!(view.stalled.resets > 0);
        assert_eq!(view.fast.resets, 0);
        let mut late = Consumer::new(&mut view.state, 2);
        late.drain(&view.state, &view.images);
        view.state.reproject(&fixture.config, false).unwrap();
        view.record();
        late.drain(&view.state, &view.images);
    }
    fixture.close();
}
