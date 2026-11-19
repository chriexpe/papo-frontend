//! Process-local live deferral state, bounded queues, fair lookup scheduling budget and canonical
//! point resolution. Event loop scheduling lives in `live`; delivery admission lives in `deliver`.

use super::{NotificationError, context, policy::Candidate, triggers};
use crate::{
    ClientError, Generation, MessageKey, ReadingSnapshot, ResourceSnapshot, Session, Uuid,
};
use std::{collections::VecDeque, sync::Arc, time::Duration};
use tokio::{sync::watch, time::Instant};

pub(super) const DEFERRED_CAPACITY: usize = 64;
/// Maximum gateway events admitted while a deferred lookup is due before granting the lookup a turn.
pub(super) const MAX_EVENTS_BEFORE_LOOKUP: usize = 8;
/// Maximum bounded 4-page turns per live target (initial turn plus resumes = 16 pages max).
pub(super) const MAX_LIVE_LOOKUP_TURNS: u8 = 4;
/// Pacing delay between consecutive resumed lookup turns so an idle loop never storms REST.
pub(super) const LOOKUP_TURN_INTERVAL: Duration = Duration::from_millis(25);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LivePoint {
    Found(MessageKey),
    Pending,
    Absent,
}

pub(super) struct DeferredLookup {
    pub generation: Generation,
    pub candidate: Candidate,
    pub turns: u8,
}

pub(super) struct LiveState {
    pub recent: VecDeque<(Generation, Candidate)>,
    pub triggers: triggers::TriggerEvidence,
    pub points: context::CanonicalPoints,
    pub lookup_queue: VecDeque<DeferredLookup>,
    pub evidence_queue: VecDeque<(Generation, Candidate)>,
    pub events_since_lookup: usize,
    pub next_lookup_at: Option<Instant>,
    pub resources_rx: watch::Receiver<Arc<ResourceSnapshot>>,
    pub reading_rx: watch::Receiver<Arc<ReadingSnapshot>>,
}

impl LiveState {
    pub fn new(session: &Session) -> Self {
        Self {
            recent: VecDeque::new(),
            triggers: triggers::TriggerEvidence::default(),
            points: context::CanonicalPoints::default(),
            lookup_queue: VecDeque::new(),
            evidence_queue: VecDeque::new(),
            events_since_lookup: 0,
            next_lookup_at: None,
            resources_rx: session.subscribe_resources(),
            reading_rx: session.subscribe_reading(),
        }
    }

    pub fn retain_generation(&mut self, generation: Generation) {
        self.recent.retain(|(item_gen, _)| *item_gen == generation);
        self.lookup_queue
            .retain(|entry| entry.generation == generation);
        self.evidence_queue
            .retain(|(item_gen, _)| *item_gen == generation);
        if self.lookup_queue.is_empty() {
            self.events_since_lookup = 0;
            self.next_lookup_at = None;
        }
    }

    pub fn clear_deferred(&mut self) {
        self.lookup_queue.clear();
        self.evidence_queue.clear();
        self.events_since_lookup = 0;
        self.next_lookup_at = None;
    }

    pub fn observe_watches(&mut self) {
        let _ = self.resources_rx.borrow_and_update();
        let _ = self.reading_rx.borrow_and_update();
    }

    /// Ensures a continuously replenished event backlog cannot starve a due deferred lookup.
    pub fn lookup_should_preempt_events(&self, now: Instant) -> bool {
        !self.lookup_queue.is_empty()
            && self.events_since_lookup >= MAX_EVENTS_BEFORE_LOOKUP
            && self.next_lookup_at.is_none_or(|due| now >= due)
    }

    pub fn note_event_while_lookup_pending(&mut self) {
        if !self.lookup_queue.is_empty() {
            self.events_since_lookup = self.events_since_lookup.saturating_add(1);
        }
    }

    pub fn note_lookup_turn(&mut self, now: Instant) {
        self.events_since_lookup = 0;
        self.next_lookup_at = if self.lookup_queue.is_empty() {
            None
        } else {
            Some(now + LOOKUP_TURN_INTERVAL)
        };
    }

    pub fn lookup_deadline(&self) -> Option<Instant> {
        (!self.lookup_queue.is_empty()).then(|| self.next_lookup_at.unwrap_or_else(Instant::now))
    }
}

fn merge_candidate(existing: &mut Candidate, candidate: Candidate) {
    if candidate.triggered && !existing.triggered {
        let point = candidate.message_point.or(existing.message_point);
        let read = existing.read || candidate.read;
        *existing = candidate;
        existing.message_point = point;
        existing.read = read;
        if let Some(point) = point {
            existing.observed_at_us = point.created_at_us;
        }
    } else {
        if existing.message_point.is_none()
            && let Some(point) = candidate.message_point
        {
            existing.message_point = Some(point);
            existing.observed_at_us = point.created_at_us;
        }
        existing.read |= candidate.read;
    }
}

/// Inserts a deferred evidence candidate while preserving boundedness and upgrading any earlier
/// non-triggered copy of the same message (for example an authoritative reply arriving after
/// its `Message` event while a channel is quarantined).
pub(super) fn enqueue_deferred(
    queue: &mut VecDeque<(Generation, Candidate)>,
    generation: Generation,
    candidate: Candidate,
    front: bool,
) -> Result<(), NotificationError> {
    if let Some((_, existing)) = queue
        .iter_mut()
        .find(|(item_gen, item)| *item_gen == generation && item.message == candidate.message)
    {
        merge_candidate(existing, candidate);
        return Ok(());
    }
    if queue.len() >= DEFERRED_CAPACITY {
        return Err(NotificationError::Overflow);
    }
    if front {
        queue.push_front((generation, candidate));
    } else {
        queue.push_back((generation, candidate));
    }
    Ok(())
}

/// Inserts a deferred lookup candidate with its turn count, preserving boundedness and target
/// continuity (front keeps the active target from thrashing `CanonicalPoints`).
pub(super) fn enqueue_lookup(
    queue: &mut VecDeque<DeferredLookup>,
    generation: Generation,
    candidate: Candidate,
    turns: u8,
    front: bool,
) -> Result<(), NotificationError> {
    if let Some(existing) = queue.iter_mut().find(|entry| {
        entry.generation == generation && entry.candidate.message == candidate.message
    }) {
        merge_candidate(&mut existing.candidate, candidate);
        existing.turns = existing.turns.max(turns);
        return Ok(());
    }
    if queue.len() >= DEFERRED_CAPACITY {
        return Err(NotificationError::Overflow);
    }
    let entry = DeferredLookup {
        generation,
        candidate,
        turns,
    };
    if front {
        queue.push_front(entry);
    } else {
        queue.push_back(entry);
    }
    Ok(())
}

pub(super) async fn resolve_live_point(
    session: &Session,
    points: &mut context::CanonicalPoints,
    generation: Generation,
    channel: Uuid,
    message: Uuid,
) -> Result<LivePoint, NotificationError> {
    let epoch = session.inner.epochs.current();
    if epoch.generation != generation {
        return Err(ClientError::Superseded.into());
    }
    let rest = crate::scoped_request::authenticated(session, &epoch).await?;
    match points
        .resolve(session, &rest, generation, channel, message)
        .await
    {
        Ok(context::Resolution::Found(point)) => Ok(LivePoint::Found(point)),
        Ok(context::Resolution::Pending) => Ok(LivePoint::Pending),
        Ok(context::Resolution::Absent)
        | Err(NotificationError::Session(ClientError::Api(
            papo_api::Error::Http {
                status: 403 | 404 | 410,
            }
            | papo_api::Error::Problem {
                status: 403 | 404 | 410,
                ..
            },
        ))) => Ok(LivePoint::Absent),
        Err(error) => Err(error),
    }
}
