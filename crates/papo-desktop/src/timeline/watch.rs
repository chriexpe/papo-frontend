//! The watch that keeps one timeline's items current, on the SDK's runtime:
//! it opens the SDK window (at the anchor when the cache holds it: live
//! when it is near the newest, focused around it when far), or takes one
//! ⌛ opened, posts the baseline, then every batch of diffs, every change
//! to the window's reading overlay (the skipped block, the arrivals) and
//! to the account's resources and to the channel's pins (fetched whenever
//! they are not known), until the timeline is dropped or the SDK closes
//! the window.
//!
//! Every post carries the timeline's key and ticket; the shell drops posts
//! from an opening it no longer holds.

use std::sync::Arc;

use futures_util::StreamExt;
use imbl::Vector;
use papo_sdk::{
    ClientHandle, Generation, HistoryClosedReason, HistoryError, Message, ReadingPosition,
    ResourceSnapshot, Session, Uuid,
};
use papo_sdk_ui::{
    ChannelPins, PinsEvent, PinsStream, Timeline, TimelineConfig, TimelineError, TimelineEvent,
    TimelineItem, TimelineReadingEvent, TimelineReadingState, TimelineUpdate, UtcOffsetProvider,
};
use rune::Tx;
use tokio::sync::oneshot;

use super::TimelineKey;
use super::anchor::Anchor;
use crate::sdk_bridge::Updates;

/// Messages per page, in the window's first page and each later one.
const PAGE_SIZE: u8 = 50;

/// A window that opened, with its first items.
pub(crate) struct Opened {
    pub(crate) handle: Timeline,
    /// Whether it follows new messages. One that does not was opened
    /// around a distant anchor.
    pub(crate) live: bool,
    pub(crate) version: u64,
    pub(crate) generation: Generation,
    pub(crate) items: Vector<Arc<TimelineItem>>,
    /// The SDK's saved reading position for the channel.
    pub(crate) saved: Option<Anchor>,
}

/// What the watch posts to the UI thread.
pub(crate) enum Posted {
    Opened(Opened),
    Update(TimelineUpdate),
    /// The window's reading overlay changed.
    Reading(TimelineReadingState),
    /// The account's resources changed (author names among them).
    Resources(Arc<ResourceSnapshot>),
    /// The channel's pins opened: their handle, which fetches them again.
    PinsOpened(ChannelPins),
    /// The channel's pins now, newest pin first; none while unknown.
    Pins(Option<Vector<Arc<Message>>>),
    /// The window could not open.
    Failed,
    /// The SDK closed the window for good.
    Closed,
}

/// Applies a post on the UI thread; returns whether the view changed.
pub(crate) type Apply<S> = fn(&mut S, TimelineKey, u64, Posted, &mut Tx) -> bool;

/// One timeline's watch.
pub(crate) struct Watch<S> {
    pub(crate) key: TimelineKey,
    pub(crate) ticket: u64,
    /// Where to open: the timeline's own place, else none and the SDK's
    /// saved reading position is used. `Newest` opens live at the newest.
    pub(crate) anchor: Option<Anchor>,
    /// A window already open (⌛'s) to take instead.
    pub(crate) window: Option<Timeline>,
    pub(crate) offsets: Arc<dyn UtcOffsetProvider>,
    pub(crate) apply: Apply<S>,
}

impl<S: 'static> Watch<S> {
    /// Runs until `stopped` fires (the timeline was dropped), the SDK
    /// closes the window, or the client shuts down.
    pub(crate) async fn run(
        self,
        client: ClientHandle,
        updates: Updates<S>,
        mut stopped: oneshot::Receiver<()>,
    ) {
        let Watch {
            key,
            ticket,
            anchor,
            window,
            offsets,
            apply,
        } = self;
        let post = move |posted: Posted| {
            updates.send_tx(move |state, tx| apply(state, key, ticket, posted, tx));
        };
        let Ok(session) = client.session(key.account) else {
            post(Posted::Failed);
            return;
        };
        let (opened, saved) = if let Some(window) = window {
            // ⌛'s window is focused around the block.
            (Ok((window, false)), session.reading_position(key.channel))
        } else {
            let opening =
                open_when_allowed(&session, key.channel, anchor, config(offsets), &mut stopped);
            let Some(opened) = opening.await else {
                return;
            };
            opened
        };
        let (handle, live, subscription, reading) = match opened.and_then(|(handle, live)| {
            let subscription = handle.subscribe()?;
            let reading = handle.subscribe_reading()?;
            Ok((handle, live, subscription, reading))
        }) {
            Ok(opened) => opened,
            Err(error) => {
                log::warn!("a timeline did not open: {error}");
                post(Posted::Failed);
                return;
            }
        };
        let mut stream = subscription.stream;
        post(Posted::Opened(Opened {
            handle,
            live,
            version: subscription.version,
            generation: subscription.generation,
            items: subscription.items,
            saved: saved.map(|position| Anchor::from_saved(Some(position))),
        }));
        post(Posted::Reading(reading.state));
        let mut reading = reading.stream;
        let mut reading_open = true;
        let mut resources = session.subscribe_resources();
        post(Posted::Resources(Arc::clone(
            &resources.borrow_and_update(),
        )));
        let mut resources_open = true;
        let mut pins = follow_pins(&session, key.channel, &post).await;
        loop {
            tokio::select! {
                biased;
                _ = &mut stopped => return,
                event = stream.next() => match event {
                    Some(TimelineEvent::Update(update)) => post(Posted::Update(update)),
                    Some(TimelineEvent::Closed(_)) | None => {
                        post(Posted::Closed);
                        return;
                    }
                    Some(_) => {}
                },
                event = reading.next(), if reading_open => match event {
                    Some(TimelineReadingEvent::Update(state)) => post(Posted::Reading(state)),
                    // The window's own closure follows on its item stream.
                    Some(TimelineReadingEvent::Closed(_)) | None => reading_open = false,
                    Some(_) => {}
                },
                changed = resources.changed(), if resources_open => {
                    if changed.is_err() {
                        // The session closed; the window's closure follows.
                        resources_open = false;
                        continue;
                    }
                    post(Posted::Resources(Arc::clone(&resources.borrow_and_update())));
                }
                event = next_pins(pins.as_mut()) => match event {
                    Some(PinsEvent::Update(update)) => {
                        if update.is_none() {
                            // Unknown again (a new connection): fetched again.
                            refresh(pins.as_ref().map(|(handle, _)| handle.clone()));
                        }
                        post(Posted::Pins(update));
                    }
                    // The window's own closure follows on its item stream.
                    Some(PinsEvent::Closed(_)) | None => pins = None,
                    Some(_) => {}
                },
            }
        }
    }
}

/// Opens the channel's pins beside its window, posts their handle and the
/// pins now, and fetches them when they are not known. None when they do
/// not open; the window works without them (no pin actions, no panel).
async fn follow_pins(
    session: &Session,
    channel: Uuid,
    post: &impl Fn(Posted),
) -> Option<(ChannelPins, PinsStream)> {
    let opened = match ChannelPins::open(session, channel).await {
        Ok(pins) => pins
            .subscribe()
            .await
            .map(|subscription| (pins, subscription)),
        Err(error) => Err(error),
    };
    let (pins, subscription) = match opened {
        Ok(opened) => opened,
        Err(error) => {
            log::debug!("a channel's pins are not followed: {error}");
            return None;
        }
    };
    post(Posted::PinsOpened(pins.clone()));
    if subscription.pins.is_none() {
        refresh(Some(pins.clone()));
    }
    post(Posted::Pins(subscription.pins));
    Some((pins, subscription.stream))
}

/// Fetches the pins on their own task: the stream posts what arrives.
pub(crate) fn refresh(pins: Option<ChannelPins>) {
    let Some(pins) = pins else {
        return;
    };
    tokio::spawn(async move {
        if let Err(error) = pins.refresh().await {
            log::debug!("a channel's pins were not fetched: {error}");
        }
    });
}

/// The next pins event; never, once the pins are gone.
async fn next_pins(pins: Option<&mut (ChannelPins, PinsStream)>) -> Option<PinsEvent> {
    match pins {
        Some((_, stream)) => stream.next().await,
        None => std::future::pending().await,
    }
}

/// The window's settings: [`PAGE_SIZE`] messages a page, days in the
/// reader's time zone. ⌛'s paging takes them too.
pub(crate) fn config(offsets: Arc<dyn UtcOffsetProvider>) -> TimelineConfig {
    TimelineConfig {
        page_size: PAGE_SIZE,
        offset_provider: offsets,
        ..TimelineConfig::default()
    }
}

/// Opens the window once the session lets it read the channel: history
/// opens only after this connection's channel list has granted it, so a
/// window asked for at startup, before the session reconnects, waits for
/// the account's resources or its connection to change and tries again.
/// Opens at `anchor`, else at the SDK's saved reading position. Returns the
/// outcome and that saved position, or `None` once `stopped` fires. A
/// session that closes meanwhile closes the window too.
async fn open_when_allowed(
    session: &Session,
    channel: Uuid,
    anchor: Option<Anchor>,
    config: TimelineConfig,
    stopped: &mut oneshot::Receiver<()>,
) -> Option<(
    Result<(Timeline, bool), TimelineError>,
    Option<ReadingPosition>,
)> {
    let mut resources = session.subscribe_resources();
    let mut connection = session.subscribe_connection();
    loop {
        let saved = session.reading_position(channel);
        let reach_back = match anchor {
            Some(anchor) => anchor.message(),
            None => saved.map(|position| position.message_id),
        };
        let opened = tokio::select! {
            biased;
            _ = &mut *stopped => return None,
            opened = open(session.clone(), channel, reach_back, config.clone()) => opened,
        };
        if !matches!(
            opened,
            Err(TimelineError::History(
                HistoryError::Unverified | HistoryError::AccessUnknown
            ))
        ) {
            return Some((opened, saved));
        }
        log::debug!("a timeline waits until its channel can be read");
        let session_open = tokio::select! {
            biased;
            _ = &mut *stopped => return None,
            changed = resources.changed() => changed.is_ok(),
            changed = connection.changed() => changed.is_ok(),
        };
        if !session_open {
            let closed = TimelineError::Closed(HistoryClosedReason::AccountClosed);
            return Some((Err(closed), saved));
        }
    }
}

/// Opens the window at `anchor` when the cache holds it, else at the
/// newest. Returns the window and whether it is live.
async fn open(
    session: Session,
    channel: Uuid,
    anchor: Option<Uuid>,
    config: TimelineConfig,
) -> Result<(Timeline, bool), TimelineError> {
    if let Some(anchor) = anchor {
        let at = match Timeline::live_at(session.clone(), channel, anchor, config.clone()).await {
            Err(TimelineError::AnchorTooFar) => {
                Timeline::focused_at(session.clone(), channel, anchor, config.clone())
                    .await
                    .map(|handle| (handle, false))
            }
            opened => opened.map(|handle| (handle, true)),
        };
        match at {
            // Deleted, or not in the cache: the newest is the best place left.
            Err(TimelineError::AnchorDeleted | TimelineError::FocusMissing) => {
                log::debug!("a timeline's anchor is gone; opening at the newest");
            }
            opened => return opened,
        }
    }
    Timeline::live(session, channel, config)
        .await
        .map(|handle| (handle, true))
}
