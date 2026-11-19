//! Shared delivery admission across live, push and background; consults reading-owned durable read
//! evidence before FULL ledger claims. Live event ingestion lives in `live`.

use super::{
    NotificationDelivery, NotificationError, ledger,
    policy::{self, Candidate, PolicyResources},
};
use crate::{
    ClientError, Generation, HistoryClosedReason, HistoryError, ReadingError, Session,
    reading::evidence::{ChannelGate, ReadEvidence, channel_gate, channel_gate_sync},
};

/// Result of evaluating one candidate against durable reading evidence, policy and the FULL ledger.
#[derive(Clone)]
pub(crate) enum DeliveryOutcome {
    /// Candidate reached a final decision; eligible candidates consumed their ledger identity.
    Handled(Option<NotificationDelivery>),
    /// Reading evidence is not ready; candidate was deferred before any ledger claim.
    Deferred,
}

pub(crate) fn record_candidate(
    record: &papo_api::wire::Notification,
) -> Result<Candidate, NotificationError> {
    Ok(Candidate {
        channel: super::id(record.channel_id.as_deref())?,
        message: super::id(record.message_id.as_deref())?,
        author: record.author_id.as_deref().and_then(|id| id.parse().ok()),
        content: record.message_content.clone().unwrap_or_default(),
        triggered: true,
        read: record.read != Some(false),
        trigger_revision: None,
        observed_at_us: crate::parse_message_timestamp_us(
            record
                .created_at
                .as_deref()
                .ok_or(NotificationError::InvalidData)?,
        )?,
        message_point: None,
    })
}

pub(crate) async fn deliver(
    session: &Session,
    generation: Generation,
    candidate: Candidate,
) -> Result<DeliveryOutcome, NotificationError> {
    let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
    deliver_with_resources(session, generation, candidate, &resources).await
}

pub(crate) async fn deliver_with_resources(
    session: &Session,
    generation: Generation,
    candidate: Candidate,
    resources: &PolicyResources,
) -> Result<DeliveryOutcome, NotificationError> {
    deliver_at(session, generation, candidate, resources, super::now_us()?).await
}

pub(super) async fn deliver_at(
    session: &Session,
    generation: Generation,
    candidate: Candidate,
    resources: &PolicyResources,
    now_us: i64,
) -> Result<DeliveryOutcome, NotificationError> {
    // The durable ledger has a 30-day retention window. This admission floor prevents
    // replay of an older live/inbox candidate after its identity has been collected.
    if candidate.observed_at_us < now_us.saturating_sub(30 * 24 * 60 * 60 * 1_000_000) {
        return Ok(DeliveryOutcome::Handled(None));
    }
    let _serial = session.inner.notifications.serial.lock().await;
    session.inner.epochs.validate(generation)?;
    if candidate.trigger_revision.is_some_and(|revision| {
        revision
            != session
                .inner
                .notifications
                .inbox_revision
                .load(std::sync::atomic::Ordering::Acquire)
    }) {
        return Ok(DeliveryOutcome::Handled(None));
    }
    let backend = session
        .inner
        .notifications
        .backend
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .filter(|policy| policy.generation == generation)
        .map(|policy| policy.config.clone())
        .ok_or(ClientError::Superseded)?;
    let settings = ledger::settings(session).await?;
    // A non-triggering ordinary message can neither deliver nor claim, so it never needs
    // read-evidence lookup or deferral on a quarantined channel (leaving room for a later reply).
    if !policy::claim_eligible(resources, &settings, &candidate) {
        return Ok(DeliveryOutcome::Handled(None));
    }
    // Backend-confirmed `read=true` independently proves suppression after canonical message
    // identity is known, avoiding head-of-line deferral when the channel is quarantined.
    if candidate.read {
        return claim_eligible(session, generation, &candidate, now_us, None).await;
    }
    let read_evidence = match crate::reading::evidence::message_read_evidence(
        session,
        generation,
        candidate.channel,
        candidate.message,
        candidate.message_point,
    )
    .await
    {
        Ok(ReadEvidence::NotReady) | Err(ReadingError::History(HistoryError::AccessUnknown)) => {
            return Ok(DeliveryOutcome::Deferred);
        }
        Ok(evidence) => evidence,
        Err(ReadingError::History(HistoryError::Closed(HistoryClosedReason::AccountClosed))) => {
            return Err(ClientError::Closed.into());
        }
        Err(ReadingError::History(HistoryError::Closed(_))) => {
            // History admission denied channel access; suppress without failing other channels.
            return claim_eligible(session, generation, &candidate, now_us, None).await;
        }
        Err(ReadingError::Session(error)) => return Err(error.into()),
        Err(ReadingError::Storage(error)) => return Err(error.into()),
        Err(_) => return Err(NotificationError::InvalidData),
    };
    let delivery = policy::decide(
        session,
        generation,
        &settings,
        &backend,
        &candidate,
        resources,
        read_evidence,
    );
    claim_eligible(session, generation, &candidate, now_us, delivery).await
}

async fn claim_eligible(
    session: &Session,
    generation: Generation,
    candidate: &Candidate,
    now_us: i64,
    delivery: Option<NotificationDelivery>,
) -> Result<DeliveryOutcome, NotificationError> {
    let will_deliver = delivery.is_some();
    // Suppressed eligible candidates also consume the identity: unmuting cannot replay them later.
    match ledger::claim_candidate(
        session,
        generation,
        candidate.channel,
        candidate.message,
        candidate.observed_at_us,
        now_us,
        will_deliver,
    )
    .await?
    {
        ledger::ClaimOutcome::Claimed => {}
        ledger::ClaimOutcome::Duplicate | ledger::ClaimOutcome::SuppressedDenied => {
            return Ok(DeliveryOutcome::Handled(None));
        }
        ledger::ClaimOutcome::Deferred => {
            return Ok(DeliveryOutcome::Deferred);
        }
    }
    session.inner.epochs.validate(generation)?;
    // Re-check effective channel admission after the awaited FULL ledger commit so a same-
    // generation revocation or quarantine cannot let an earlier delivery decision escape.
    if will_deliver {
        match channel_gate(&session.inner, candidate.channel, generation).await {
            ChannelGate::Proceed => {}
            ChannelGate::Denied(HistoryClosedReason::AccountClosed) => {
                return Err(ClientError::Closed.into());
            }
            ChannelGate::Denied(_) | ChannelGate::Quarantined => {
                return Ok(DeliveryOutcome::Handled(None));
            }
        }
    }
    Ok(DeliveryOutcome::Handled(delivery))
}

pub(crate) fn handoff(
    session: &Session,
    delivery: NotificationDelivery,
) -> Result<(), NotificationError> {
    session.inner.epochs.apply(delivery.target.generation, || {
        match channel_gate_sync(
            &session.inner,
            delivery.target.channel,
            delivery.target.generation,
        ) {
            ChannelGate::Proceed => session
                .inner
                .notifications
                .deliveries
                .try_send(delivery)
                .map_err(|_| NotificationError::Overflow),
            ChannelGate::Denied(HistoryClosedReason::AccountClosed) => {
                Err(ClientError::Closed.into())
            }
            ChannelGate::Denied(_) | ChannelGate::Quarantined => Ok(()),
        }
    })?
}
