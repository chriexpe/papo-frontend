//! Server read-mark extraction and deleted-anchor newest-page resolution.
//! Coalescing resource installs and cache unread refreshes live in `reading::reconcile`.

use super::{ReadPoint, ReadState};
use crate::{
    ClientError, ResourceSnapshot, Session, read_points::ServerReadPoint,
    reconcile::ResourceConnection,
};
use papo_api::{HistoryQuery, Uuid, wire::ChannelLastMessage};

pub(super) struct Mark {
    pub(super) channel: Uuid,
    pub(super) server: Option<ServerReadPoint>,
    pub(super) head: Option<ReadPoint>,
}

pub(super) fn marks(snapshot: &ResourceSnapshot) -> Result<Vec<Mark>, ClientError> {
    snapshot
        .channels
        .iter()
        .map(|channel| {
            (
                channel.id.as_deref(),
                channel.last_read_message.as_deref(),
                channel.last_read_at.as_deref(),
                channel.last_message.as_ref(),
            )
        })
        .chain(snapshot.dms.iter().map(|dm| {
            (
                Some(dm.id.as_str()),
                dm.last_read_message.as_deref(),
                dm.last_read_at.as_deref(),
                dm.last_message.as_ref(),
            )
        }))
        .map(|(channel, message, recorded_at, head)| {
            let channel = channel
                .ok_or(ClientError::ResourceData)?
                .parse()
                .map_err(|_| ClientError::ResourceData)?;
            let server = message
                .map(|message| {
                    Ok::<_, ClientError>(ServerReadPoint {
                        message_id: message.parse().map_err(|_| ClientError::ResourceData)?,
                        recorded_at: recorded_at.map(str::to_owned),
                    })
                })
                .transpose()?;
            Ok(Mark {
                channel,
                server,
                head: head.map(head_point).transpose()?,
            })
        })
        .collect()
}

pub(super) fn head_point(head: &ChannelLastMessage) -> Result<ReadPoint, ClientError> {
    ReadPoint::parse(head.created_at.as_deref(), head.id.as_deref())
        .map_err(|_| ClientError::ResourceData)
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
    head: Option<ReadPoint>,
) -> Result<Option<ReadPoint>, ClientError> {
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
        let point = ReadPoint::message(message).map_err(|_| ClientError::ResourceData)?;
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
    if page
        .channel_id
        .as_deref()
        .and_then(|id| id.parse::<Uuid>().ok())
        != Some(channel)
    {
        return Err(ClientError::ResourceData);
    }
    for message in page.messages.unwrap_or_default() {
        if message
            .channel_id
            .as_deref()
            .and_then(|id| id.parse::<Uuid>().ok())
            != Some(channel)
        {
            return Err(ClientError::ResourceData);
        }
        let point = ReadPoint::message(&message).map_err(|_| ClientError::ResourceData)?;
        if point.message_id == server.message_id {
            return Ok(Some(point));
        }
    }
    Ok(None)
}
