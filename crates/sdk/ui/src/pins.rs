//! A channel's pinned messages as they change, newest pin first. The pin list, its fetch and
//! its live events are history's (`HistorySnapshot::pins`); pinning and unpinning are
//! `OperationService`; the panel is the client's.

use std::sync::Arc;

use futures_util::{Stream, StreamExt, stream};
use imbl::Vector;
use papo_sdk::{
    HistoryClosedReason, HistoryError, HistoryHandle, HistoryUpdate, Message, Session, Uuid,
};
use tokio::sync::watch;

/// One change of a channel's pins.
#[derive(Clone)]
#[non_exhaustive]
pub enum PinsEvent {
    /// The pins now, newest pin first, or `None` while they are not known in this connection
    /// (after a reconnect, until they are fetched again).
    Update(Option<Vector<Arc<Message>>>),
    /// Terminal: the channel's history closed; no later event follows.
    Closed(HistoryClosedReason),
}

/// Pull-driven pin changes; no task or runtime is created.
pub type PinsStream = std::pin::Pin<Box<dyn Stream<Item = PinsEvent> + Send>>;

/// The pins when subscribed, then every change.
pub struct PinsSubscription {
    /// The pins now, as in [`PinsEvent::Update`].
    pub pins: Option<Vector<Arc<Message>>>,
    /// Each later change, then a terminal close.
    pub stream: PinsStream,
}

/// One channel's pins.
#[derive(Clone)]
pub struct ChannelPins {
    history: HistoryHandle,
}

impl ChannelPins {
    /// Opens `channel`'s pins under the same access rules as its history.
    /// # Errors
    /// Returns the history open errors (`Session::history`).
    pub async fn open(session: &Session, channel: Uuid) -> Result<Self, HistoryError> {
        Ok(Self {
            history: session.history(channel).await?,
        })
    }

    /// Fetches the channel's pin list, so the pins become known in this connection and live
    /// pin events keep them current. Call it when the pins are `None`, and whenever they are
    /// shown: a pin of a message the client never loaded arrives only with a fetch (the
    /// backend's pin event names no channel).
    /// # Errors
    /// Returns the pin fetch errors (`HistoryHandle::pins`).
    pub async fn refresh(&self) -> Result<(), HistoryError> {
        self.history.pins().await.map(|_| ())
    }

    /// The pins now and every change after.
    /// # Errors
    /// Returns the history subscription errors (`HistoryHandle::subscribe`).
    pub async fn subscribe(&self) -> Result<PinsSubscription, HistoryError> {
        let subscription = self.history.subscribe().await?;
        let pins = subscription.snapshot.pins.clone();
        let changes = stream::unfold(
            Some((subscription.updates, pins.clone())),
            |state| async move {
                let (mut updates, mut last) = state?;
                loop {
                    let next = next_pins(&mut updates).await;
                    match next {
                        Err(reason) => return Some((PinsEvent::Closed(reason), None)),
                        Ok(pins) if !same(pins.as_ref(), last.as_ref()) => {
                            last.clone_from(&pins);
                            return Some((PinsEvent::Update(pins), Some((updates, last))));
                        }
                        Ok(_) => {}
                    }
                }
            },
        );
        Ok(PinsSubscription {
            pins,
            stream: Box::pin(changes.fuse()),
        })
    }
}

/// The pins of the next history update, or why history closed.
async fn next_pins(
    updates: &mut watch::Receiver<HistoryUpdate>,
) -> Result<Option<Vector<Arc<Message>>>, HistoryClosedReason> {
    if updates.changed().await.is_err() {
        return Err(HistoryClosedReason::AccountClosed);
    }
    match &*updates.borrow_and_update() {
        HistoryUpdate::Reset(snapshot) => Ok(snapshot.pins.clone()),
        HistoryUpdate::Closed(reason) => Err(*reason),
        _ => Err(HistoryClosedReason::AccountClosed),
    }
}

/// The same pins: unchanged payloads keep their `Arc` across publications.
fn same(left: Option<&Vector<Arc<Message>>>, right: Option<&Vector<Arc<Message>>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right.iter())
                    .all(|(left, right)| Arc::ptr_eq(left, right))
        }
        _ => false,
    }
}
