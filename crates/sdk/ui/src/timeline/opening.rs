//! What a window loads to set its first bounds, and the canonical subscription it starts from:
//! the newest page, an exact cursor's neighborhood, or, for a message known only by its ID (a
//! saved reading position), the newest page plus a reach back to it or its own neighborhood.
//! Locating an ID is the history owner's (`HistorySnapshot::locate`); bounds, projection and
//! publication stay in `state`.

use std::sync::Arc;

use papo_sdk::{
    HistoryCursor, HistoryHandle, HistoryOrder, HistoryQuery, HistorySnapshot, HistorySubscription,
    LocatedMessage, Message, Uuid,
};

use super::TimelineError;

/// How far back a live window may reach to an anchor, in pages counted from the newest cached
/// message. Every message between the anchor and the newest joins a live window, which is
/// projected (and parsed) whole on open and on each later change; a farther anchor opens in a
/// bounded focused neighborhood instead.
pub(super) const MAX_REACH_BACK_PAGES: usize = 20;

/// How a window opens.
pub(super) enum Opening {
    /// The newest page; the window follows new messages.
    Live,
    /// The newest page, then a cached anchor and one page older than it; followed live.
    LiveAt(Uuid),
    /// An exact cursor's neighborhood; the window never follows new messages.
    Focused(HistoryCursor),
    /// A cached anchor's neighborhood, at any distance; never follows new messages.
    FocusedAt(Uuid),
}

impl Opening {
    /// Whether the window's newest bound follows future messages.
    pub(super) fn is_live(&self) -> bool {
        matches!(self, Self::Live | Self::LiveAt(_))
    }

    /// The message an opening by ID is anchored on.
    pub(super) fn anchor(&self) -> Option<Uuid> {
        match self {
            Self::LiveAt(anchor) | Self::FocusedAt(anchor) => Some(*anchor),
            Self::Live | Self::Focused(_) => None,
        }
    }
}

/// The messages that bound a new window, and the canonical subscription it starts from.
pub(super) struct Loaded {
    pub messages: Vec<Arc<Message>>,
    pub subscription: HistorySubscription,
}

/// Loads what `opening` needs. A focused cursor must already be a valid key.
pub(super) async fn load(
    history: &HistoryHandle,
    opening: &Opening,
    page_size: u8,
) -> Result<Loaded, TimelineError> {
    let messages = match opening {
        Opening::Live => newest(history, page_size).await?,
        Opening::Focused(cursor) => neighborhood(history, cursor, page_size).await?,
        Opening::LiveAt(anchor) | Opening::FocusedAt(anchor) => {
            // The newest page also restores the durable cache the anchor is looked up in.
            let newest = newest(history, page_size).await?;
            // Subscribed before the lookup, so a message found is in the window's history.
            let subscription = history.subscribe().await?;
            let (message, newer) = held(&subscription.snapshot, *anchor)?;
            let cursor = message.cursor().ok_or(TimelineError::InvalidCursor)?;
            let messages = if matches!(opening, Opening::LiveAt(_)) {
                if newer >= MAX_REACH_BACK_PAGES * usize::from(page_size) {
                    return Err(TimelineError::AnchorTooFar);
                }
                let mut messages = newest;
                messages.extend(older(history, &cursor, page_size).await?);
                messages.push(message);
                messages
            } else {
                neighborhood(history, &cursor, page_size).await?
            };
            return Ok(Loaded {
                messages,
                subscription,
            });
        }
    };
    let subscription = history.subscribe().await?;
    Ok(Loaded {
        messages,
        subscription,
    })
}

async fn newest(
    history: &HistoryHandle,
    page_size: u8,
) -> Result<Vec<Arc<Message>>, TimelineError> {
    page(history, None, HistoryOrder::Desc, page_size).await
}

/// Up to one page strictly older than `cursor`.
async fn older(
    history: &HistoryHandle,
    cursor: &HistoryCursor,
    page_size: u8,
) -> Result<Vec<Arc<Message>>, TimelineError> {
    page(history, Some(cursor), HistoryOrder::Desc, page_size).await
}

async fn page(
    history: &HistoryHandle,
    cursor: Option<&HistoryCursor>,
    order: HistoryOrder,
    page_size: u8,
) -> Result<Vec<Arc<Message>>, TimelineError> {
    Ok(history
        .page(HistoryQuery {
            cursor,
            order,
            limit: Some(page_size),
        })
        .await?
        .messages)
}

/// An exact cursor's neighborhood: a page older than it, then a page forward from there.
/// `FocusMissing` when the server's neighborhood lacks the target.
async fn neighborhood(
    history: &HistoryHandle,
    focus: &HistoryCursor,
    page_size: u8,
) -> Result<Vec<Arc<Message>>, TimelineError> {
    let mut messages = older(history, focus, page_size).await?;
    // The pin has no GET-by-ID route. Fetching forward from its nearest older row includes
    // the target using only exact server-supplied cursors, including equal-time UUID ties.
    let before = messages.first().and_then(|message| message.cursor());
    messages.extend(page(history, before.as_ref(), HistoryOrder::Asc, page_size).await?);
    if !messages
        .iter()
        .any(|message| message.cursor().as_ref() == Some(focus))
    {
        return Err(TimelineError::FocusMissing);
    }
    Ok(messages)
}

/// The held anchor and how many cached messages are newer than it.
fn held(snapshot: &HistorySnapshot, anchor: Uuid) -> Result<(Arc<Message>, usize), TimelineError> {
    match snapshot.locate(anchor)? {
        LocatedMessage::Held { index, .. } => Ok((
            Arc::clone(&snapshot.messages[index]),
            snapshot.messages.len() - 1 - index,
        )),
        LocatedMessage::Deleted => Err(TimelineError::AnchorDeleted),
        LocatedMessage::Unknown => Err(TimelineError::FocusMissing),
    }
}
