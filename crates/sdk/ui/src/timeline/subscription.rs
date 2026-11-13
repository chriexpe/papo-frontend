//! Atomic timeline subscriptions and wakeups across history, reading and outgoing; projection stays in `WindowState`.

use super::{
    Arc, HistoryClosedReason, Timeline, TimelineError, TimelineEvent, TimelineSubscription,
    TimelineUpdate,
};
use crate::list_subscription;
use futures_util::{StreamExt, stream};

impl Timeline {
    /// Capture `(version, items, stream)` under one lock; overflow emits a coherent Reset.
    /// # Errors
    /// Returns terminal closure, invalid authoritative cursor or exhausted version.
    pub fn subscribe(&self) -> Result<TimelineSubscription, TimelineError> {
        let mut state = self.inner.lock();
        state.refresh(&self.inner.config)?;
        let version = state.publisher.version;
        let generation = state.publisher.generation;
        let items = state.publisher.items.clone();
        let receiver = state
            .publisher
            .subscribe(self.inner.config.subscriber_capacity);
        let history = state.history.clone();
        let reading = state.reading.clone();
        let outgoing = state.outgoing.clone();
        let owner = Arc::clone(&self.inner);
        let changes = stream::unfold(
            (owner, receiver, history, reading, outgoing),
            |(owner, mut receiver, mut history, mut reading, mut outgoing)| async move {
                loop {
                    let _ = owner.lock().refresh(&owner.config);
                    let closed = tokio::select! {
                        event = receiver.next() => {
                            return event.map(|event| {
                                (map_event(event), (owner, receiver, history, reading, outgoing))
                            });
                        }
                        change = history.changed() => change.is_err(),
                        change = reading.changed() => change.is_err(),
                        change = outgoing_changed(outgoing.as_mut()) => change.is_err(),
                    };
                    if closed {
                        owner.lock().close(HistoryClosedReason::AccountClosed);
                    }
                }
            },
        );
        Ok(TimelineSubscription {
            version,
            generation,
            items,
            stream: Box::pin(changes.fuse()),
        })
    }
}

fn map_event(
    event: list_subscription::Event<
        Arc<crate::TimelineItem>,
        papo_sdk::Generation,
        HistoryClosedReason,
    >,
) -> TimelineEvent {
    match event {
        list_subscription::Event::Update {
            version,
            generation,
            diffs,
        } => TimelineEvent::Update(TimelineUpdate {
            version,
            generation,
            diffs,
        }),
        list_subscription::Event::Closed(reason) => TimelineEvent::Closed(reason),
    }
}

async fn outgoing_changed(
    outgoing: Option<&mut tokio::sync::watch::Receiver<Arc<papo_sdk::OutgoingSnapshot>>>,
) -> Result<(), tokio::sync::watch::error::RecvError> {
    match outgoing {
        Some(rx) => rx.changed().await,
        None => std::future::pending().await,
    }
}
