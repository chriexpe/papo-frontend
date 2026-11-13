//! One account dispatcher, durable claim/receipt boundaries and conservative ambiguity; never matches content/time.

use super::OutgoingDispatchPhase;
use super::{
    Arc, ClientError, MessageTarget, Operation, OperationFailure, OperationState, OutgoingError,
    OutgoingFault, OutgoingService, OutgoingSnapshot, Uuid, persistence::Outcome,
};
use crate::{ConnectionState, Session};
use papo_api::RestClient;
use std::{collections::BTreeSet, sync::atomic::Ordering};

pub(crate) fn start(session: Session) {
    if session.is_closed() || session.inner.outgoing.running.swap(true, Ordering::AcqRel) {
        return;
    }
    let token = session.inner.tasks.token();
    session.client.inner.runtime.clone().spawn(async move {
        let _token = token;
        dispatch_loop(&session).await;
        session
            .inner
            .outgoing
            .running
            .store(false, Ordering::Release);
    });
}

async fn dispatch_loop(session: &Session) {
    let mut connection = session.subscribe_connection();
    let service = OutgoingService {
        session: session.clone(),
    };
    let mut retry_attempt = 0_u32;
    loop {
        if session.is_closed() {
            break;
        }
        let result = run_drain(&service).await;
        let retry = result.is_err() || has_pending_projection(session);
        if let Err(error) = result {
            record_fault(session, &error).await;
        }
        let retry_delay = session
            .client
            .inner
            .config
            .backoff
            .delay(retry_attempt, session.client.inner.config.jitter.as_ref());
        retry_attempt = if retry {
            retry_attempt.saturating_add(1)
        } else {
            0
        };
        if !wait_for_retry(session, &mut connection, retry_delay, retry).await {
            break;
        }
    }
}

async fn run_drain(service: &OutgoingService) -> Result<(), OutgoingError> {
    service.ensure().await?;
    drain(service).await
}

fn has_pending_projection(session: &Session) -> bool {
    session
        .inner
        .outgoing
        .image
        .borrow()
        .operations
        .iter()
        .any(|op| op.state == OperationState::Sent && !op.history_projected)
}

async fn record_fault(session: &Session, error: &OutgoingError) {
    let _guard = session.inner.outgoing.serial.lock().await;
    let fault = match error {
        OutgoingError::Storage(_) | OutgoingError::InvalidData => OutgoingFault::Storage,
        _ => OutgoingFault::Unavailable,
    };
    session.inner.outgoing.image.send_if_modified(|image| {
        if image.fault == Some(fault) {
            return false;
        }
        let mut next = (**image).clone();
        next.fault = Some(fault);
        next.version = next.version.saturating_add(1);
        *image = Arc::new(next);
        true
    });
}

async fn wait_for_retry(
    session: &Session,
    connection: &mut tokio::sync::watch::Receiver<crate::ConnectionStatus>,
    retry_delay: std::time::Duration,
    retry: bool,
) -> bool {
    tokio::select! {
        biased;
        () = session.inner.root.cancelled() => false,
        () = session.inner.outgoing.wake.notified() => true,
        result = connection.changed() => result.is_ok(),
        () = tokio::time::sleep(retry_delay), if retry => true,
    }
}

fn next(snapshot: &OutgoingSnapshot) -> Option<Arc<Operation>> {
    let mut channels = BTreeSet::new();
    snapshot
        .operations
        .iter()
        .find(|operation| {
            // Completed sends do not block followers in the same channel.
            if operation.state == OperationState::Sent {
                return false;
            }
            if !channels.insert(operation.channel_id) || operation.state != OperationState::Queued {
                return false;
            }
            operation.depends_on.is_none_or(|id| {
                snapshot.operations.iter().any(|op| {
                    op.id == id && op.state == OperationState::Sent && op.message_id.is_some()
                })
            })
        })
        .cloned()
}

async fn drain(service: &OutgoingService) -> Result<(), OutgoingError> {
    let session = &service.session;
    loop {
        service.check()?;
        if session.inner.connection.borrow().state != ConnectionState::Online {
            return Ok(());
        }
        super::projection::reconcile_receipts(service).await?;
        let epoch = session.inner.epochs.current();
        let operation = {
            let snapshot = session.inner.outgoing.image.borrow();
            next(&snapshot)
        };
        let Some(operation) = operation else {
            return Ok(());
        };
        // Credentials and transport preparation have not dispatched, so failures leave Queued.
        let rest = prepare(session, &epoch).await?;
        let target = resolve(service, &operation)?;
        probe(session, operation.id, OutgoingDispatchPhase::Queued).await;
        claim_sending(service, &epoch, operation.id).await?;
        probe(session, operation.id, OutgoingDispatchPhase::Claimed).await;
        let result = execute_send(session, &epoch, &rest, &operation, target).await;
        if result.is_ok() {
            probe(session, operation.id, OutgoingDispatchPhase::Response).await;
        }
        let (state, outcome, failure) = classify(result);
        if session.is_closed() {
            return Ok(());
        } // Sending recovers Unknown on startup.
        finish_attempt(
            service,
            epoch.generation,
            &operation,
            state,
            outcome,
            failure,
        )
        .await?;
        probe(session, operation.id, OutgoingDispatchPhase::Committed).await;
        if state == OperationState::Queued {
            return Err(ClientError::Api(papo_api::Error::NotDispatched).into());
        }
    }
}

async fn claim_sending(
    service: &OutgoingService,
    epoch: &crate::generation::Epoch,
    operation_id: Uuid,
) -> Result<(), OutgoingError> {
    let session = &service.session;
    let _guard = session.inner.outgoing.serial.lock().await;
    session.inner.epochs.validate(epoch.generation)?;
    session
        .client
        .inner
        .registry
        .outgoing_transition(
            session.clone(),
            operation_id,
            OperationState::Queued,
            OperationState::Sending,
            None,
            None,
        )
        .await?;
    service.publish().await
}

async fn execute_send(
    session: &Session,
    epoch: &crate::generation::Epoch,
    rest: &RestClient,
    operation: &Operation,
    target: Option<Uuid>,
) -> Result<Outcome, papo_api::Error> {
    let timeout = session.client.inner.config.request_timeout;
    let request = super::request::send(rest, operation, target, session.account().user());
    tokio::select! {
        biased;
        () = session.inner.root.cancelled() => Err(papo_api::Error::Transport),
        () = epoch.cancel.cancelled() => Err(papo_api::Error::Transport),
        result = tokio::time::timeout(timeout, request) => {
            result.unwrap_or(Err(papo_api::Error::Transport))
        }
    }
}

async fn finish_attempt(
    service: &OutgoingService,
    generation: crate::Generation,
    operation: &Operation,
    state: OperationState,
    outcome: Option<Outcome>,
    failure: Option<OperationFailure>,
) -> Result<(), OutgoingError> {
    let session = &service.session;
    let guard = session.inner.outgoing.serial.lock().await;
    // Intent belongs to the account, not a socket generation: record a confirmed receipt
    // after reconnect, but never publish to a revoked/replaced account handle.
    let acknowledged = outcome.as_ref().and_then(|value| value.message.clone());
    session
        .client
        .inner
        .registry
        .outgoing_transition(
            session.clone(),
            operation.id,
            OperationState::Sending,
            state,
            outcome,
            failure,
        )
        .await?;
    let projection_result = if state == OperationState::Sent {
        super::projection::finish(service, operation, generation, acknowledged).await
    } else {
        Ok(())
    };
    service.publish().await?;
    let collection_result = if state == OperationState::Sent {
        service.collect().await
    } else {
        Ok(())
    };
    drop(guard);
    collection_result?;
    projection_result
}

fn classify(
    result: Result<Outcome, papo_api::Error>,
) -> (OperationState, Option<Outcome>, Option<OperationFailure>) {
    match result {
        Ok(outcome) => (OperationState::Sent, Some(outcome), None),
        Err(papo_api::Error::NotDispatched) => (OperationState::Queued, None, None),
        Err(papo_api::Error::Problem { status, .. } | papo_api::Error::Http { status })
            if (400..500).contains(&status) && status != 408 =>
        {
            (
                OperationState::Failed,
                None,
                Some(OperationFailure::Rejected(status)),
            )
        }
        // Transport, decode, timeout, generation cancellation and 5xx may follow server acceptance.
        Err(_) => (OperationState::Unknown, None, None),
    }
}

fn resolve(
    service: &OutgoingService,
    operation: &Operation,
) -> Result<Option<Uuid>, OutgoingError> {
    match operation.kind.target() {
        None => Ok(None),
        Some(MessageTarget::Remote(id)) => Ok(Some(id)),
        Some(MessageTarget::Local(id)) => service
            .session
            .inner
            .outgoing
            .image
            .borrow()
            .operations
            .iter()
            .find(|op| op.id == id && op.state == OperationState::Sent)
            .and_then(|op| op.message_id)
            .map(Some)
            .ok_or(OutgoingError::InvalidData),
    }
}
async fn prepare(
    session: &Session,
    epoch: &crate::generation::Epoch,
) -> Result<RestClient, OutgoingError> {
    let secret = tokio::select! {
        biased;
        () = session.inner.root.cancelled() => return Err(ClientError::Closed.into()),
        () = epoch.cancel.cancelled() => return Err(ClientError::Superseded.into()),
        result = session.client.inner.secrets.read(session.inner.key, &epoch.cancel) => {
            result.map_err(ClientError::from)?
        }
    }
    .ok_or(ClientError::SignedOut)?;
    let rest = session
        .client
        .inner
        .transport(&session.inner.address, Some(secret.expose()))?;
    session.inner.epochs.validate(epoch.generation)?;
    Ok(rest)
}

#[cfg(feature = "test-support")]
async fn probe(session: &Session, id: Uuid, phase: OutgoingDispatchPhase) {
    if let Some(probe) = &session.client.inner.config.outgoing_probe {
        tokio::select! {
            biased;
            () = session.inner.root.cancelled() => {}
            () = probe.reached(id, phase) => {}
        }
    }
}

#[cfg(not(feature = "test-support"))]
fn probe(_session: &Session, _id: Uuid, _phase: OutgoingDispatchPhase) -> std::future::Ready<()> {
    std::future::ready(())
}

#[cfg(test)]
mod tests;
