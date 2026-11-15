//! Behind-head live arrival counting and two-stop down-arrow target projection, asked of
//! history's coverage queries and its one authorship rule. Separate catch-up boundary state
//! and overlay streams live in `timeline_reading`.

use papo_sdk::{
    HistoryCoverage, HistoryCursor, HistorySnapshot, MessageKey, Uuid, authored_by_other,
};

use super::TimelineError;

/// Newly arrived foreign messages behind a timeline viewport and their two down-arrow targets.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct TimelineArrivals {
    /// Count of unseen foreign messages newer than this window's exposed frontier.
    pub count: u64,
    /// First down-arrow stop: oldest unseen foreign arrival.
    pub oldest_unseen: Option<HistoryCursor>,
    /// Second down-arrow stop: newest live tail message on the channel.
    pub live_head: Option<HistoryCursor>,
}

impl std::fmt::Debug for TimelineArrivals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TimelineArrivals")
            .field("count", &self.count)
            .field("has_oldest_unseen", &self.oldest_unseen.is_some())
            .field("has_live_head", &self.live_head.is_some())
            .finish()
    }
}

fn newest_key(raw: &HistorySnapshot) -> Result<Option<MessageKey>, TimelineError> {
    Ok(raw
        .messages
        .last()
        .map(|message| MessageKey::from_message(message))
        .transpose()?)
}

/// Nothing unknown follows the newest held message in this generation.
pub(super) fn is_live_complete(raw: &HistorySnapshot) -> Result<bool, TimelineError> {
    live_complete(&raw.coverage()?, raw)
}

fn live_complete(
    coverage: &HistoryCoverage<'_>,
    raw: &HistorySnapshot,
) -> Result<bool, TimelineError> {
    Ok(coverage.range(newest_key(raw)?, None)?.complete)
}

pub(super) fn initial_live_tail(
    raw: &HistorySnapshot,
) -> Result<Option<MessageKey>, TimelineError> {
    if !is_live_complete(raw)? {
        return Ok(None);
    }
    newest_key(raw)
}

pub(super) fn tail_before_live_arrivals(
    raw: &HistorySnapshot,
) -> Result<Option<MessageKey>, TimelineError> {
    if !is_live_complete(raw)? {
        return Ok(None);
    }
    if let Some(cursor) = &raw.live_arrival_start {
        let floor = MessageKey::from_cursor(cursor)?;
        return tail_before_point(raw, floor);
    }
    initial_live_tail(raw)
}

/// The newest held message strictly before `floor`.
pub(super) fn tail_before_point(
    raw: &HistorySnapshot,
    floor: MessageKey,
) -> Result<Option<MessageKey>, TimelineError> {
    let before = raw.coverage()?.range(None, Some(floor))?.messages;
    let Some(index) = before.end.checked_sub(1) else {
        return Ok(None);
    };
    Ok(Some(MessageKey::from_message(&raw.messages[index])?))
}

pub(super) fn compute_arrivals(
    self_user: Uuid,
    raw: &HistorySnapshot,
    window_seen_to: Option<MessageKey>,
) -> Result<TimelineArrivals, TimelineError> {
    let coverage = raw.coverage()?;
    if !live_complete(&coverage, raw)? {
        return Ok(TimelineArrivals::default());
    }
    let cutoff = window_seen_to
        .map(|point| coverage.resolve(point).map(|resolved| resolved.key))
        .transpose()?;
    let mut count = 0u64;
    let mut oldest_unseen = None;
    for index in coverage.range(cutoff, None)?.messages {
        let message = &raw.messages[index];
        if !authored_by_other(message, self_user) {
            continue;
        }
        count += 1;
        if oldest_unseen.is_none() {
            oldest_unseen = message.cursor();
        }
    }
    let live_head = (count > 0)
        .then(|| raw.messages.last().and_then(|message| message.cursor()))
        .flatten();
    Ok(TimelineArrivals {
        count,
        oldest_unseen,
        live_head,
    })
}
