//! The watch that keeps one timeline's items current, on the SDK's runtime:
//! it opens the SDK window (at the anchor when the cache holds it: live when
//! it is near the newest, focused around it when far), posts the baseline, then every batch of diffs and every change to the
//! account's resources, until the timeline is dropped or the SDK closes the
//! window.
//!
//! Every post carries the timeline's key and ticket; the shell drops posts
//! from an opening it no longer holds.

use std::sync::Arc;

use futures_util::StreamExt;
use imbl::Vector;
use papo_sdk::{
    ClientHandle, Generation, HistoryError, ReadingPosition, ResourceSnapshot, Session, Uuid,
};
use papo_sdk_ui::{
    Timeline, TimelineConfig, TimelineError, TimelineEvent, TimelineItem, TimelineUpdate,
    UtcOffsetProvider,
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
    /// The account's resources changed (author names among them).
    Resources(Arc<ResourceSnapshot>),
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
    /// The message to reach back to: the timeline's own anchor, else none
    /// and the SDK's saved position is used.
    pub(crate) anchor: Option<Uuid>,
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
        let config = TimelineConfig {
            page_size: PAGE_SIZE,
            offset_provider: offsets,
            ..TimelineConfig::default()
        };
        let Some((opened, saved)) =
            open_when_allowed(&session, key.channel, anchor, config, &mut stopped).await
        else {
            return;
        };
        let (handle, live, subscription) = match opened.and_then(|(handle, live)| {
            let subscription = handle.subscribe()?;
            Ok((handle, live, subscription))
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
        let mut resources = session.subscribe_resources();
        post(Posted::Resources(Arc::clone(
            &resources.borrow_and_update(),
        )));
        let mut resources_open = true;
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
                changed = resources.changed(), if resources_open => {
                    if changed.is_err() {
                        // The session closed; the window's closure follows.
                        resources_open = false;
                        continue;
                    }
                    post(Posted::Resources(Arc::clone(&resources.borrow_and_update())));
                }
            }
        }
    }
}

/// Opens the window once the session lets it read the channel: history
/// opens only after this connection's channel list has granted it, so a
/// window asked for at startup, before the session reconnects, waits for
/// the account's resources or its connection to change and tries again.
/// Opens at `anchor`, else at the SDK's saved reading position. Returns the
/// outcome and that saved position, or `None` once `stopped` fires.
async fn open_when_allowed(
    session: &Session,
    channel: Uuid,
    anchor: Option<Uuid>,
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
        let reach_back = anchor.or(saved.map(|position| position.message_id));
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
        tokio::select! {
            biased;
            _ = &mut *stopped => return None,
            changed = resources.changed() => changed.ok()?,
            changed = connection.changed() => changed.ok()?,
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
