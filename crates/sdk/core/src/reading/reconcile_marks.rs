//! Server read-mark extraction and deleted-anchor newest-page resolution.
//! Coalescing resource installs and cache unread refreshes live in `reading::reconcile`.

use super::ReadState;
use crate::MessageKey;
use crate::{
    ClientError, ResourceSnapshot, Session, read_points::ServerReadPoint,
    reconcile::ResourceConnection,
};
use papo_api::{HistoryQuery, Uuid};

pub(super) struct Mark {
    pub(super) channel: Uuid,
    pub(super) server: Option<ServerReadPoint>,
    pub(super) head: Option<MessageKey>,
}

pub(super) fn marks(snapshot: &ResourceSnapshot) -> Vec<Mark> {
    snapshot
        .channels
        .iter()
        .map(|channel| {
            (
                channel.id,
                channel.last_read_message,
                channel.last_read_at.as_ref(),
                channel.last_message.as_ref(),
            )
        })
        .chain(snapshot.dms.iter().map(|dm| {
            (
                dm.id,
                dm.last_read_message,
                dm.last_read_at.as_ref(),
                dm.last_message.as_ref(),
            )
        }))
        .map(|(channel, message, recorded_at, head)| Mark {
            channel,
            server: message.map(|message_id| ServerReadPoint {
                message_id,
                recorded_at: recorded_at.cloned(),
            }),
            head: head.map(head_point),
        })
        .collect()
}

pub(super) fn head_point(head: &crate::MessageHead) -> MessageKey {
    MessageKey {
        message_id: head.id,
        created_at_us: head.created_at_us,
    }
}

/// A deleted anchor with a previously resolved key stays ordered. The pinned protocol
/// has no lookup-by-id endpoint: at most one newest-page read is attempted when absent.
/// Failure to find it is unknown, never a fabricated ordering from `recorded_at`.
pub(super) async fn resolve(
    session: &Session,
    connection: &ResourceConnection,
    generation: crate::Generation,
    channel: Uuid,
    server: &ServerReadPoint,
    state: &ReadState,
    head: Option<MessageKey>,
) -> Result<Option<MessageKey>, ClientError> {
    for point in [state.local_point, state.uploaded_point, head]
        .into_iter()
        .flatten()
    {
        if point.message_id == server.message_id {
            return Ok(Some(point));
        }
    }
    let history = session.history(channel).await?;
    let cached = history.subscribe().await?.snapshot;
    for message in &cached.messages {
        let point = MessageKey::from_message(message).map_err(|_| ClientError::ResourceData)?;
        if point.message_id == server.message_id {
            return Ok(Some(point));
        }
    }
    let page = connection
        .client
        .inner
        .request(connection.rest.messages(
            channel,
            &HistoryQuery {
                limit: Some(100),
                ..HistoryQuery::default()
            },
        ))
        .await?;
    session.inner.epochs.validate(generation)?;
    let validated = crate::history::validate_descending_page(channel, None, 100, &page)
        .map_err(|_| ClientError::ResourceData)?;
    Ok(validated
        .points
        .into_iter()
        .find(|point| point.message_id == server.message_id))
}
