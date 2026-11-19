//! Process-local live deferral state, bounded queues and canonical point resolution.
//! Event loop scheduling and gateway ingestion live in `live`; delivery admission lives in `deliver`.

use super::{NotificationError, context, policy::Candidate, triggers};
use crate::{
    ClientError, Generation, MessageKey, ReadingSnapshot, ResourceSnapshot, Session, Uuid,
};
use std::{collections::VecDeque, sync::Arc};
use tokio::sync::watch;

pub(super) const DEFERRED_CAPACITY: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LivePoint {
    Found(MessageKey),
    Pending,
    Absent,
}

pub(super) struct LiveState {
    pub recent: VecDeque<(Generation, Candidate)>,
    pub triggers: triggers::TriggerEvidence,
    pub points: context::CanonicalPoints,
    pub lookup_queue: VecDeque<(Generation, Candidate)>,
    pub evidence_queue: VecDeque<(Generation, Candidate)>,
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
            resources_rx: session.subscribe_resources(),
            reading_rx: session.subscribe_reading(),
        }
    }

    pub fn retain_generation(&mut self, generation: Generation) {
        self.recent.retain(|(item_gen, _)| *item_gen == generation);
        self.lookup_queue
            .retain(|(item_gen, _)| *item_gen == generation);
        self.evidence_queue
            .retain(|(item_gen, _)| *item_gen == generation);
    }

    pub fn observe_watches(&mut self) {
        let _ = self.resources_rx.borrow_and_update();
        let _ = self.reading_rx.borrow_and_update();
    }
}

/// Inserts a deferred candidate while preserving boundedness and upgrading any earlier
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
