//! Reading-owned durable read-evidence queries over `state.sqlite` read points and current-generation
//! reading state. Leaves notification policy/claims to `notifications` and access decisions to
//! `history::access`.

use super::positions::ChannelFence;
use crate::{
    Generation, HistoryClosedReason, HistoryError, MessageKey, ReadingError, Session,
    history::access::ChannelAdmission, read_points::UploadedPoint, session::SessionInner,
};
use papo_api::Uuid;
use std::collections::BTreeMap;

/// Reading-owned recount dependencies omitted from the public `ReadState` projection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ChannelBoundary {
    pub head: Option<MessageKey>,
    pub unresolved: Option<Uuid>,
}

/// Snapshots existing boundary inputs without SQL or new storage. Callers fence the associated
/// reading-image generation; these descriptors are invalidation inputs, never read authority.
pub(crate) async fn channel_boundaries(session: &SessionInner) -> BTreeMap<Uuid, ChannelBoundary> {
    let heads = session.reading.heads.lock().await;
    let unresolved = session.reading.unresolved.lock().await;
    let mut boundaries: BTreeMap<_, _> = heads
        .iter()
        .map(|(&channel, &head)| {
            (
                channel,
                ChannelBoundary {
                    head: Some(head),
                    unresolved: None,
                },
            )
        })
        .collect();
    for (&channel, mark) in unresolved.iter() {
        boundaries.entry(channel).or_default().unresolved = Some(mark.message_id);
    }
    boundaries
}

/// Explicit read-evidence decision for one candidate message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadEvidence {
    /// Durable ordered points or exact-ID equality with an unresolved server mark proves read.
    Read,
    /// Available evidence does not mark this message read.
    Unread,
    /// Channel evidence is quarantined, badge boundary evidence is missing, or the candidate
    /// lacks a key needed to compare against an existing ordered point; callers must defer.
    NotReady,
}

/// One fenced channel query; candidate evaluation performs no I/O.
#[derive(Debug, Default)]
pub(crate) struct ChannelReadEvidence {
    pub(crate) ready: bool,
    boundary_known: bool,
    point: Option<MessageKey>,
    head: Option<MessageKey>,
    exact_ids: [Option<Uuid>; 7],
}

impl ChannelReadEvidence {
    pub(crate) fn point(&self) -> Option<MessageKey> {
        self.point
    }

    pub(crate) fn head(&self) -> Option<MessageKey> {
        self.head
    }

    /// A current-generation channel image establishes even an empty boundary, unless a server
    /// mark remains unresolved. Durable row absence alone never establishes this readiness.
    pub(crate) fn boundary_known(&self) -> bool {
        self.boundary_known
    }

    /// Applies the caller's coverage-resolved legacy key before the same pure evaluation.
    pub(crate) fn with_point(mut self, point: Option<MessageKey>) -> Self {
        self.point = point;
        self
    }

    pub(crate) fn message(&self, message: Uuid, point: Option<MessageKey>) -> ReadEvidence {
        if !self.ready {
            return ReadEvidence::NotReady;
        }
        evaluate_evidence(message, point, self.point, &self.exact_ids)
    }

    /// Missing boundary evidence can prove a candidate read, never unread for badge counting.
    pub(crate) fn badge_message(&self, message: Uuid, point: Option<MessageKey>) -> ReadEvidence {
        let evidence = self.message(message, point);
        if evidence == ReadEvidence::Read || self.boundary_known() {
            evidence
        } else {
            ReadEvidence::NotReady
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChannelGate {
    Proceed,
    Quarantined,
    Denied(HistoryClosedReason),
}

fn evaluate_channel_gate(
    admission: Option<ChannelAdmission>,
    fence: Option<ChannelFence>,
) -> ChannelGate {
    if let Some(ChannelAdmission::Denied(reason)) = admission {
        return ChannelGate::Denied(reason);
    }
    if let Some(fence) = fence {
        return match fence {
            ChannelFence::Closed(reason) => ChannelGate::Denied(reason),
            ChannelFence::Unknown => ChannelGate::Quarantined,
        };
    }
    if matches!(admission, Some(ChannelAdmission::Quarantined)) {
        return ChannelGate::Quarantined;
    }
    ChannelGate::Proceed
}

pub(crate) async fn channel_gate(
    session: &SessionInner,
    channel: Uuid,
    generation: Generation,
) -> ChannelGate {
    let admission = session.history.admission.read().await.channel(channel);
    let fence = session
        .reading
        .positions
        .check_current_channel(channel, generation);
    evaluate_channel_gate(admission, fence)
}

/// Synchronous channel fence check for FULL writer closures, platform handoff, final background
/// delivery filters and target validation. Reuses reading positions and history admission without
/// introducing any notification-owned permission state; fails closed as `Quarantined` while the
/// admission write lock is contended (for example during `seal` or `deny` before `positions`
/// updates).
pub(crate) fn channel_gate_sync(
    session: &SessionInner,
    channel: Uuid,
    generation: Generation,
) -> ChannelGate {
    let admission = match session.history.admission.try_read() {
        Ok(admissions) => admissions.channel(channel),
        Err(_) => Some(ChannelAdmission::Quarantined),
    };
    let fence = session
        .reading
        .positions
        .check_current_channel(channel, generation);
    evaluate_channel_gate(admission, fence)
}

/// Evaluates whether `message` in `channel` is read using durable local/uploaded/known points,
/// current-generation reading state and exact-ID equality with unresolved server marks.
/// Never compares server acknowledgement timestamps (`recorded_at`) as message order.
/// # Errors
/// Returns generation/account closure (`ReadingError::Session`), channel revocation from history
/// admission or reading fences (`ReadingError::History(HistoryError::Closed(_))`), or storage
/// corruption (`ReadingError::Storage`).
pub(crate) async fn message_read_evidence(
    session: &Session,
    generation: Generation,
    channel: Uuid,
    message: Uuid,
    message_point: Option<MessageKey>,
) -> Result<ReadEvidence, ReadingError> {
    Ok(channel_read_evidence(session, generation, channel)
        .await?
        .message(message, message_point))
}

/// Loads durable and current-generation evidence once per channel; quarantine returns unready.
/// # Errors
/// Retains notification query's lifecycle, denied-access and storage errors.
pub(crate) async fn channel_read_evidence(
    session: &Session,
    generation: Generation,
    channel: Uuid,
) -> Result<ChannelReadEvidence, ReadingError> {
    session.inner.epochs.validate(generation)?;
    match channel_gate(&session.inner, channel, generation).await {
        ChannelGate::Denied(reason) => {
            return Err(ReadingError::History(HistoryError::Closed(reason)));
        }
        ChannelGate::Quarantined => return Ok(ChannelReadEvidence::default()),
        ChannelGate::Proceed => {}
    }

    let stored = session
        .client
        .inner
        .registry
        .load_channel_read_points(session.account(), channel)
        .await?;

    let (same_generation, has_channel, published_local, published_uploaded) = {
        let image = session.inner.reading.image.borrow();
        let same_generation = image.generation == generation;
        let state = image.channels.get(&channel).filter(|_| same_generation);
        (
            same_generation,
            state.is_some(),
            state.and_then(|state| state.local_point),
            state.and_then(|state| state.uploaded_point),
        )
    };
    let head = if same_generation {
        let heads = session.inner.reading.heads.lock().await;
        heads.get(&channel).copied()
    } else {
        None
    };
    let unresolved_id = if same_generation {
        session
            .inner
            .reading
            .unresolved
            .lock()
            .await
            .get(&channel)
            .map(|mark| mark.message_id)
    } else {
        None
    };

    // Re-check generation and channel fences after every `.await` so a concurrent revocation,
    // quarantine or epoch change while reading SQLite or waiting on `unresolved` cannot slip by.
    session.inner.epochs.validate(generation)?;
    match channel_gate(&session.inner, channel, generation).await {
        ChannelGate::Denied(reason) => {
            return Err(ReadingError::History(HistoryError::Closed(reason)));
        }
        ChannelGate::Quarantined => return Ok(ChannelReadEvidence::default()),
        ChannelGate::Proceed => {}
    }

    let durable_local = stored.as_ref().and_then(|points| points.local);
    let durable_uploaded = stored.as_ref().and_then(|points| points.uploaded.as_ref());
    let durable_uploaded_ordered = durable_uploaded.and_then(UploadedPoint::point);
    let durable_known = durable_uploaded.and_then(|uploaded| uploaded.known_point);
    let durable_server_id = durable_uploaded.map(|uploaded| uploaded.server.message_id);

    let ordered_point = durable_local
        .max(durable_uploaded_ordered)
        .max(published_local)
        .max(published_uploaded);
    let exact_ids = [
        durable_local.map(|point| point.message_id),
        durable_uploaded_ordered.map(|point| point.message_id),
        durable_known.map(|point| point.message_id),
        durable_server_id,
        published_local.map(|point| point.message_id),
        published_uploaded.map(|point| point.message_id),
        unresolved_id,
    ];
    Ok(ChannelReadEvidence {
        ready: true,
        boundary_known: has_channel
            && unresolved_id.is_none()
            && durable_uploaded.is_none_or(|uploaded| uploaded.created_at_us.is_some()),
        point: ordered_point,
        head,
        exact_ids,
    })
}

fn evaluate_evidence(
    message: Uuid,
    message_point: Option<MessageKey>,
    ordered_point: Option<MessageKey>,
    exact_ids: &[Option<Uuid>],
) -> ReadEvidence {
    if exact_ids.iter().flatten().any(|&id| id == message) {
        return ReadEvidence::Read;
    }
    match (ordered_point, message_point) {
        (Some(read_point), Some(candidate_point)) => {
            if candidate_point <= read_point {
                ReadEvidence::Read
            } else {
                ReadEvidence::Unread
            }
        }
        (Some(_), None) => ReadEvidence::NotReady,
        (None, _) => ReadEvidence::Unread,
    }
}

#[cfg(test)]
#[path = "channel_evidence_tests.rs"]
mod channel_tests;
#[cfg(test)]
#[path = "evidence_tests.rs"]
mod tests;
