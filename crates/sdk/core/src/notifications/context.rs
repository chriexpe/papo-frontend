//! Bounded canonical message-time evidence for notification retention; never grants history coverage or reads.
use super::NotificationError;
use crate::{Generation, HistoryCursor, HistoryQuery, ReadPoint, Session, Uuid};
use std::collections::BTreeMap;

#[derive(Default)]
struct ChannelPoints {
    points: BTreeMap<Uuid, ReadPoint>,
    cursor: Option<HistoryCursor>,
    target: Option<Uuid>,
    complete: bool,
}
#[derive(Default)]
pub(crate) struct CanonicalPoints {
    scope: Option<(Generation, Uuid)>,
    state: ChannelPoints,
}
pub(crate) enum Resolution {
    Found(ReadPoint),
    Pending,
    Absent,
}
impl Resolution {
    pub fn point(self) -> Option<ReadPoint> {
        match self {
            Self::Found(point) => Some(point),
            Self::Pending | Self::Absent => None,
        }
    }
}
impl CanonicalPoints {
    pub async fn resolve(
        &mut self,
        session: &Session,
        rest: &papo_api::RestClient,
        generation: Generation,
        channel: Uuid,
        message: Uuid,
    ) -> Result<Resolution, NotificationError> {
        if self.scope != Some((generation, channel)) {
            self.scope = Some((generation, channel));
            self.state = ChannelPoints::default();
        }
        let state = &mut self.state;
        if let Some(point) = state.points.get(&message) {
            return Ok(Resolution::Found(*point));
        }
        // A continuation belongs to one target. A different uncached target starts at newest.
        if state.target != Some(message) {
            state.cursor = None;
            state.complete = false;
            state.target = Some(message);
        }
        let mut pages = 0;
        while !state.points.contains_key(&message) && !state.complete && pages < 4 {
            let response = session
                .client
                .inner
                .request(rest.messages(
                    channel,
                    &HistoryQuery {
                        cursor: state.cursor.as_ref(),
                        limit: Some(100),
                        ..HistoryQuery::default()
                    },
                ))
                .await?;
            session.inner.epochs.validate(generation)?;
            if response
                .channel_id
                .as_deref()
                .and_then(|id| id.parse().ok())
                != Some(channel)
            {
                return Err(NotificationError::InvalidData);
            }
            let messages = response.messages.unwrap_or_default();
            if messages.len() > 100 {
                return Err(NotificationError::InvalidData);
            }
            let mut previous = state
                .cursor
                .as_ref()
                .map(|cursor| ReadPoint::parse(Some(&cursor.created_at), Some(&cursor.id)))
                .transpose()
                .map_err(|_| NotificationError::InvalidData)?;
            let mut points = BTreeMap::new();
            for message in &messages {
                if message.channel_id.as_deref().and_then(|id| id.parse().ok()) != Some(channel) {
                    return Err(NotificationError::InvalidData);
                }
                let point =
                    ReadPoint::message(message).map_err(|_| NotificationError::InvalidData)?;
                if previous.is_some_and(|previous| point >= previous) {
                    return Err(NotificationError::InvalidData);
                }
                previous = Some(point);
                points.insert(point.message_id, point);
            }
            // Keep only the current page. Progress uses the exact backend cursor, not cached rows.
            state.points = points;
            pages += 1;
            state.complete = response.has_more != Some(true);
            if !state.complete {
                state.cursor = Some(
                    messages
                        .last()
                        .and_then(crate::Message::cursor)
                        .ok_or(NotificationError::InvalidData)?,
                );
            }
        }
        Ok(match state.points.get(&message) {
            Some(point) => Resolution::Found(*point),
            None if state.complete => Resolution::Absent,
            None => Resolution::Pending,
        })
    }
}
