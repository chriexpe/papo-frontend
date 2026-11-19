//! Reading-owned durable read-evidence queries over `state.sqlite` read points and current-generation
//! reading state. Leaves notification policy/claims to `notifications` and access decisions to
//! `history::access`.

use super::positions::ChannelFence;
use crate::{
    Generation, HistoryClosedReason, HistoryError, MessageKey, ReadingError, Session,
    history::access::ChannelAdmission, read_points::UploadedPoint, session::SessionInner,
};
use papo_api::Uuid;

/// Explicit read-evidence decision for one candidate message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReadEvidence {
    /// Durable ordered points or exact-ID equality with an unresolved server mark proves read.
    Read,
    /// Available evidence does not mark this message read.
    Unread,
    /// Channel evidence is quarantined or lacks the candidate key needed to compare against an
    /// existing ordered read point; callers must defer before claiming.
    NotReady,
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
    session.inner.epochs.validate(generation)?;
    match channel_gate(&session.inner, channel, generation).await {
        ChannelGate::Denied(reason) => {
            return Err(ReadingError::History(HistoryError::Closed(reason)));
        }
        ChannelGate::Quarantined => return Ok(ReadEvidence::NotReady),
        ChannelGate::Proceed => {}
    }

    let stored = session
        .client
        .inner
        .registry
        .load_channel_read_points(session.account(), channel)
        .await?;

    let (same_generation, published_local, published_uploaded) = {
        let image = session.inner.reading.image.borrow();
        let same_generation = image.generation == generation;
        if same_generation {
            let state = image.channels.get(&channel);
            (
                true,
                state.and_then(|state| state.local_point),
                state.and_then(|state| state.uploaded_point),
            )
        } else {
            (false, None, None)
        }
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
        ChannelGate::Quarantined => return Ok(ReadEvidence::NotReady),
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
    Ok(evaluate_evidence(
        message,
        message_point,
        ordered_point,
        &exact_ids,
    ))
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
#[path = "evidence_tests.rs"]
mod tests;
