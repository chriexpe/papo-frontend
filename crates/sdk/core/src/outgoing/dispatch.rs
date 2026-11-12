//! One account dispatcher, durable claim/receipt boundaries and conservative ambiguity; never matches content/time.

use super::OutgoingDispatchPhase;
use super::{
    Arc, ClientError, MessageTarget, Operation, OperationFailure, OperationKind, OperationState,
    OutgoingError, OutgoingFault, OutgoingService, OutgoingSnapshot, Uuid, persistence::Outcome,
};
use crate::{ConnectionState, Session};
use papo_api::{
    MessageAttachment, MessageUpload, RestClient,
    wire::{ReactionRequest, UpdateMessageRequest},
};
use std::{collections::BTreeSet, sync::atomic::Ordering};

pub(crate) fn start(session: Session) {
    if session.is_closed() || session.inner.outgoing.running.swap(true, Ordering::AcqRel) {
        return;
    }
    let token = session.inner.tasks.token();
    session.client.inner.runtime.clone().spawn(async move {
        let _token = token;
        let mut connection = session.subscribe_connection();
        let service = OutgoingService {
            session: session.clone(),
        };
        loop {
            if session.is_closed() {
                break;
            }
            let result=async {service.ensure().await?;drain(&service).await}.await;
            let retry=result.is_err() || session.inner.outgoing.image.borrow().operations.iter().any(|op|op.state==OperationState::Sent && !op.history_projected);
            if let Err(error)=result {
                let _guard = session.inner.outgoing.serial.lock().await;
                let fault=match error {OutgoingError::Storage(_)|OutgoingError::InvalidData=>OutgoingFault::Storage,_=>OutgoingFault::Unavailable};
                session.inner.outgoing.image.send_if_modified(|image| {
                    if image.fault==Some(fault) {return false;}
                    let mut next=(**image).clone(); next.fault=Some(fault); next.version=next.version.saturating_add(1); *image=Arc::new(next);true
                });
            }
            tokio::select! {
                biased;
                ()=session.inner.root.cancelled()=>break,
                ()=session.inner.outgoing.wake.notified()=>{},
                result=connection.changed()=>{if result.is_err(){break;}},
                ()=tokio::time::sleep(session.client.inner.config.backoff.delay(0,session.client.inner.config.jitter.as_ref())), if retry=>{},
            }
        }
        session
            .inner
            .outgoing
            .running
            .store(false, Ordering::Release);
    });
}

fn next(snapshot: &OutgoingSnapshot) -> Option<Arc<Operation>> {
    let mut channels = BTreeSet::new();
    snapshot
        .operations
        .iter()
        .find(|operation| {
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
        {
            let guard = session.inner.outgoing.serial.lock().await;
            session.inner.epochs.validate(epoch.generation)?;
            session
                .client
                .inner
                .registry
                .outgoing_transition(
                    session.clone(),
                    operation.id,
                    OperationState::Queued,
                    OperationState::Sending,
                    None,
                    None,
                )
                .await?;
            service.publish().await?;
            drop(guard);
        }
        probe(session, operation.id, OutgoingDispatchPhase::Claimed).await;
        let result = tokio::select! {
            biased;
            ()=session.inner.root.cancelled()=>Err(papo_api::Error::Transport),
            ()=epoch.cancel.cancelled()=>Err(papo_api::Error::Transport),
            result=tokio::time::timeout(session.client.inner.config.request_timeout,send(&rest,&operation,target,session.account().user()))=>result.unwrap_or(Err(papo_api::Error::Transport)),
        };
        if result.is_ok() {
            probe(session, operation.id, OutgoingDispatchPhase::Response).await;
        }
        let (state, outcome, failure) = classify(result);
        if session.is_closed() {
            return Ok(());
        } // Sending recovers Unknown on startup.
        let guard = session.inner.outgoing.serial.lock().await;
        // Intent belongs to the account, not a socket generation: record a confirmed receipt
        // after reconnect, but never publish to a revoked/replaced account handle.
        let acknowledged = outcome.as_ref().and_then(|outcome| outcome.message.clone());
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
            super::projection::finish(service, &operation, epoch.generation, acknowledged).await
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
        projection_result?;
        probe(session, operation.id, OutgoingDispatchPhase::Committed).await;
        if state == OperationState::Queued {
            return Err(ClientError::Api(papo_api::Error::NotDispatched).into());
        }
    }
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
async fn send(
    rest: &RestClient,
    operation: &Operation,
    target: Option<Uuid>,
    author: Uuid,
) -> Result<Outcome, papo_api::Error> {
    let message = match operation.kind.as_ref() {
        OperationKind::Message(message) => {
            let attachments: Vec<_> = message
                .attachments
                .iter()
                .map(|file| MessageAttachment {
                    filename: &file.filename,
                    bytes: &file.bytes,
                })
                .collect();
            let embeds =
                serde_json::to_string(&message.embeds).map_err(|_| papo_api::Error::Decode)?;
            let reply = target.map(|id| id.to_string());
            Some(
                rest.create_message(&MessageUpload {
                    channel_id: &operation.channel_id.to_string(),
                    content: Some(&message.content),
                    reply_to: reply.as_deref(),
                    embeds: (!message.embeds.is_empty()).then_some(embeds.as_str()),
                    attachments: &attachments,
                })
                .await?,
            )
        }
        OperationKind::Edit {
            content, embeds, ..
        } => Some(
            rest.update_message(
                target.ok_or(papo_api::Error::Decode)?,
                &UpdateMessageRequest {
                    content: content.clone(),
                    embeds: embeds.clone(),
                },
            )
            .await?,
        ),
        OperationKind::Delete { .. } => {
            rest.delete_message(target.ok_or(papo_api::Error::Decode)?)
                .await?;
            None
        }
        OperationKind::Reaction {
            emoji_id,
            unicode,
            add,
            ..
        } => {
            let request = ReactionRequest {
                emoji_id: emoji_id.map(|id| id.to_string()),
                unicode: unicode.clone(),
            };
            let id = target.ok_or(papo_api::Error::Decode)?;
            if *add {
                rest.add_reaction(operation.channel_id, id, &request)
                    .await?;
            } else {
                rest.remove_reaction(operation.channel_id, id, &request)
                    .await?;
            }
            None
        }
    };
    if let Some(message) = message {
        let expected = matches!(operation.kind.as_ref(), OperationKind::Edit { .. })
            .then_some(target)
            .flatten();
        super::receipt::message(message, operation.channel_id, author, expected)
    } else {
        Ok(Outcome {
            projected: true,
            message_id: target,
            message: None,
        })
    }
}

#[cfg(feature = "test-support")]
async fn probe(session: &Session, id: Uuid, phase: OutgoingDispatchPhase) {
    #[cfg(feature = "test-support")]
    if let Some(probe) = &session.client.inner.config.outgoing_probe {
        tokio::select! { biased; ()=session.inner.root.cancelled()=>{}, ()=probe.reached(id,phase)=>{} }
    }
    let _ = (session, id, phase);
}

async fn prepare(
    session: &Session,
    epoch: &crate::generation::Epoch,
) -> Result<RestClient, OutgoingError> {
    let secret=tokio::select! {
            biased;
            ()=session.inner.root.cancelled()=>return Err(ClientError::Closed.into()),
            ()=epoch.cancel.cancelled()=>return Err(ClientError::Superseded.into()),
            result=session.client.inner.secrets.read(session.inner.key,&epoch.cancel)=>result.map_err(ClientError::from)?,
        }.ok_or(ClientError::SignedOut)?;
    let rest = session
        .client
        .inner
        .transport(&session.inner.address, Some(secret.expose()))?;
    session.inner.epochs.validate(epoch.generation)?;
    Ok(rest)
}

#[cfg(not(feature = "test-support"))]
fn probe(_session: &Session, _id: Uuid, _phase: OutgoingDispatchPhase) -> std::future::Ready<()> {
    std::future::ready(())
}

#[cfg(test)]
mod tests {
    //! Pure admission tests leave sockets and clocks to integration proofs.
    use super::*;
    use crate::OutgoingMessage;
    #[test]
    fn unresolved_channel_head_blocks_followers_but_not_another_channel() {
        let make = |id, channel, state| {
            Arc::new(Operation {
                id: Uuid::from_u128(id),
                channel_id: Uuid::from_u128(channel),
                kind: Arc::new(OperationKind::Message(OutgoingMessage::default())),
                state,
                attempts: 0,
                depends_on: None,
                failure: None,
                message_id: None,
                message: None,
                history_projected: true,
                created_at_us: 0,
            })
        };
        let mut snapshot = OutgoingSnapshot {
            account: None,
            version: 0,
            fault: None,
            operations: vec![
                make(1, 1, OperationState::Unknown),
                make(2, 1, OperationState::Queued),
                make(3, 2, OperationState::Queued),
            ],
        };
        assert_eq!(next(&snapshot).unwrap().id, Uuid::from_u128(3));
        snapshot.operations.pop();
        assert!(next(&snapshot).is_none());
    }
}
