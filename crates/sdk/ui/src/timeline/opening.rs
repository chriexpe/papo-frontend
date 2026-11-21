//! What a window loads to set its first bounds: the newest page, an exact cursor's neighborhood,
//! or the newest page plus a reach back to a cached anchor. Bounds, projection and publication
//! stay in `state`; canonical paging and caching stay in the SDK.

use std::sync::Arc;

use papo_sdk::{
    HistoryCursor, HistoryHandle, HistoryOrder, HistoryQuery, HistorySnapshot, Message, Uuid,
};

use super::TimelineError;

/// How far back an anchor may be, in pages counted from the newest cached message. Every message
/// between the anchor and the newest joins the window, which is projected (and parsed) whole on
/// open and on each later change, so the reach is bounded like a seek is.
pub(super) const MAX_REACH_BACK_PAGES: usize = 20;

/// How a window opens.
pub(super) enum Opening {
    /// The newest page; the window follows new messages.
    Live,
    /// The newest page, then the anchor and one page older than it; followed live.
    LiveAt(Uuid),
    /// An exact cursor's neighborhood; the window never follows new messages.
    Focused(HistoryCursor),
}

impl Opening {
    /// Whether the window's newest bound follows future messages.
    pub(super) fn is_live(&self) -> bool {
        !matches!(self, Self::Focused(_))
    }
}

/// The messages that bound a window before it subscribes. A focused opening fails with
/// `FocusMissing` when the server's neighborhood lacks its target.
pub(super) async fn first_messages(
    history: &HistoryHandle,
    opening: &Opening,
    page_size: u8,
) -> Result<Vec<Arc<Message>>, TimelineError> {
    let focus = match opening {
        Opening::Focused(cursor) => Some(cursor),
        Opening::Live | Opening::LiveAt(_) => None,
    };
    let first = history
        .page(HistoryQuery {
            cursor: focus,
            order: HistoryOrder::Desc,
            limit: Some(page_size),
        })
        .await?;
    let mut messages = first.messages;
    if let Some(focus) = focus {
        // The pin has no GET-by-ID route. Fetching forward from its nearest older row includes
        // the target using only exact server-supplied cursors, including equal-time UUID ties.
        let before = messages.first().and_then(|message| message.cursor());
        let after = history
            .page(HistoryQuery {
                cursor: before.as_ref(),
                order: HistoryOrder::Asc,
                limit: Some(page_size),
            })
            .await?;
        messages.extend(after.messages);
        if !messages
            .iter()
            .any(|message| message.cursor().as_ref() == Some(focus))
        {
            return Err(TimelineError::FocusMissing);
        }
    }
    Ok(messages)
}

/// The anchor and up to one page older than it, for a window whose raw history is `snapshot`.
/// Only a cached, undeleted message within [`MAX_REACH_BACK_PAGES`] of the newest can anchor:
/// its exact server cursor comes from the cache, since there is no GET-by-ID route.
pub(super) async fn reach_back(
    history: &HistoryHandle,
    snapshot: &HistorySnapshot,
    anchor: Uuid,
    page_size: u8,
) -> Result<Vec<Arc<Message>>, TimelineError> {
    let reach = MAX_REACH_BACK_PAGES * usize::from(page_size);
    let Some(message) = snapshot.messages.iter().rev().take(reach).find(|message| {
        message
            .id
            .as_deref()
            .and_then(|id| Uuid::parse_str(id).ok())
            == Some(anchor)
    }) else {
        return Err(TimelineError::FocusMissing);
    };
    let cursor = message.cursor().ok_or(TimelineError::InvalidCursor)?;
    let older = history
        .page(HistoryQuery {
            cursor: Some(&cursor),
            order: HistoryOrder::Desc,
            limit: Some(page_size),
        })
        .await?;
    let mut messages = older.messages;
    messages.push(Arc::clone(message));
    Ok(messages)
}
