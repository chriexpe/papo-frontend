//! Atomic list baselines and bounded subscriber queues; domain projections own items and source watches.

use std::{
    collections::VecDeque,
    pin::Pin,
    sync::{Arc, Mutex, Weak},
    task::{Context, Poll},
};

use eyeball_im::VectorDiff;
use futures_util::{Stream, task::AtomicWaker};
use imbl::Vector;

use crate::list_diff;

/// A complete list transition, or a sticky terminal lifecycle event.
pub(crate) enum Event<T: Clone, G, C> {
    Update {
        version: u64,
        generation: G,
        diffs: Vec<VectorDiff<T>>,
    },
    Closed(C),
}

struct QueueState<T: Clone, G, C> {
    events: VecDeque<Event<T, G, C>>,
    overflowed: bool,
}

struct Queue<T: Clone, G, C> {
    state: Mutex<QueueState<T, G, C>>,
    waker: AtomicWaker,
    capacity: usize,
}

/// Only the projection's lock may access this publisher: subscription capture and publication
/// are one critical section. Receivers never block a producer or retain a projection owner.
pub(crate) struct Publisher<T: Clone, G, C> {
    pub version: u64,
    pub generation: G,
    pub items: Vector<T>,
    subscribers: Vec<Weak<Queue<T, G, C>>>,
    closed: Option<C>,
}

impl<T: Clone + PartialEq, G: Copy, C: Clone> Publisher<T, G, C> {
    pub fn new(version: u64, generation: G, items: Vector<T>) -> Self {
        Self {
            version,
            generation,
            items,
            subscribers: Vec::new(),
            closed: None,
        }
    }

    pub fn subscribe(&mut self, capacity: usize) -> Receiver<T, G, C> {
        let queue = Arc::new(Queue {
            state: Mutex::new(QueueState {
                events: self.closed.clone().map(Event::Closed).into_iter().collect(),
                overflowed: false,
            }),
            waker: AtomicWaker::new(),
            capacity: capacity.max(1),
        });
        self.subscribers
            .retain(|subscriber| subscriber.strong_count() != 0);
        self.subscribers.push(Arc::downgrade(&queue));
        Receiver {
            queue,
            ended: false,
        }
    }

    pub fn publish<K: Eq>(
        &mut self,
        version: u64,
        generation: G,
        items: Vector<T>,
        reset: bool,
        identity: impl Fn(&T) -> K,
    ) {
        if self.closed.is_some() {
            return;
        }
        let diffs = if reset {
            vec![VectorDiff::Reset {
                values: items.clone(),
            }]
        } else {
            list_diff::between(&self.items, &items, identity)
        };
        self.version = version;
        self.generation = generation;
        self.items = items;
        self.subscribers.retain(|subscriber| {
            let Some(queue) = subscriber.upgrade() else {
                return false;
            };
            let mut state = queue
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let diffs = if state.overflowed || state.events.len() >= queue.capacity {
                // Until this reset is consumed, later publications replace it rather than
                // retaining a stale reset plus a second chain of incremental edits.
                state.events.clear();
                state.overflowed = true;
                vec![VectorDiff::Reset {
                    values: self.items.clone(),
                }]
            } else {
                diffs.clone()
            };
            state.events.push_back(Event::Update {
                version,
                generation,
                diffs,
            });
            drop(state);
            queue.waker.wake();
            true
        });
    }

    pub fn close(&mut self, reason: C) {
        if self.closed.is_some() {
            return;
        }
        self.closed = Some(reason.clone());
        self.items.clear();
        self.subscribers.retain(|subscriber| {
            let Some(queue) = subscriber.upgrade() else {
                return false;
            };
            let mut state = queue
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.events.clear();
            state.events.push_back(Event::Closed(reason.clone()));
            drop(state);
            queue.waker.wake();
            true
        });
    }
}

/// A bounded queue which releases its registration without requiring an asynchronous drop.
pub(crate) struct Receiver<T: Clone, G, C> {
    queue: Arc<Queue<T, G, C>>,
    ended: bool,
}

impl<T: Clone, G, C> Unpin for Receiver<T, G, C> {}

impl<T: Clone, G, C> Stream for Receiver<T, G, C> {
    type Item = Event<T, G, C>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.ended {
            return Poll::Ready(None);
        }
        self.queue.waker.register(cx.waker());
        let event = {
            let mut state = self
                .queue
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let event = state.events.pop_front();
            if event.is_some() {
                state.overflowed = false;
            }
            event
        };
        if matches!(event, Some(Event::Closed(_))) {
            self.ended = true;
        }
        event.map_or(Poll::Pending, |event| Poll::Ready(Some(event)))
    }
}

#[cfg(test)]
mod tests;
