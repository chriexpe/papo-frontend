//! Bounded canonical message-time evidence for notification retention; never grants history coverage or reads.
use super::NotificationError;
use crate::{Generation, HistoryCursor, HistoryQuery, MessageKey, Session, Uuid};
use std::collections::BTreeMap;

#[derive(Default)]
struct ChannelPoints {
    points: BTreeMap<Uuid, MessageKey>,
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
    Found(MessageKey),
    Pending,
    Absent,
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
            let validated = crate::history::validate_descending_page(
                channel,
                state.cursor.as_ref(),
                100,
                &response,
            )?;
            let mut points = BTreeMap::new();
            for point in validated.points {
                points.insert(point.message_id, point);
            }
            // Keep only the current page. Progress uses the exact backend cursor, not cached rows.
            state.points = points;
            pages += 1;
            state.complete = validated.next_cursor.is_none();
            state.cursor = validated.next_cursor;
        }
        Ok(match state.points.get(&message) {
            Some(point) => Resolution::Found(*point),
            None if state.complete => Resolution::Absent,
            None => Resolution::Pending,
        })
    }
}
