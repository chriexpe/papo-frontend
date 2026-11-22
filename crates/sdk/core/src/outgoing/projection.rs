//! Retries local confirmed-receipt projection; never dispatches or infers sends from content.

use super::{OperationState, OutgoingError, OutgoingService};
use papo_api::gateway::{KnownEvent, MessageDeleteEvent, MessageEditEvent};

pub(super) async fn reconcile_receipts(service: &OutgoingService) -> Result<(), OutgoingError> {
    let session = &service.session;
    let _guard = session.inner.outgoing.serial.lock().await;
    let pending: Vec<_> = session
        .inner
        .outgoing
        .image
        .borrow()
        .operations
        .iter()
        .filter(|op| op.state == OperationState::Sent && !op.history_projected)
        .cloned()
        .collect();
    let mut ids = Vec::new();
    let mut failed = None;
    let generation = session.inner.epochs.current().generation;
    for op in pending {
        let Some(id) = op.message_id else {
            continue;
        };
        // One receipt that cannot be projected yet must not hold back the others.
        match project(session, generation, &op, id).await {
            Ok(true) => ids.push(op.id),
            Ok(false) => {}
            Err(error) => {
                failed.get_or_insert(error);
            }
        }
    }
    if !ids.is_empty() {
        mark(service, ids).await?;
        service.publish().await?;
    }
    failed.map_or(Ok(()), |error| Err(error.into()))
}

/// Projects one confirmed receipt again: an edit or deletion is settled (settling twice
/// reaches the same state), a create is found in or restored to history. False when there is
/// nothing to project yet.
async fn project(
    session: &crate::Session,
    generation: crate::Generation,
    op: &super::Operation,
    id: super::Uuid,
) -> Result<bool, crate::ClientError> {
    if let Some(event) = settlement(op, id, op.message.as_deref()) {
        crate::history::live::settle(session, generation, op.channel_id, &event).await?;
        return Ok(true);
    }
    if crate::history::live::contains_receipt(session, op.channel_id, id).await {
        return Ok(true);
    }
    if let Some(message) = &op.message {
        // Persisted receipts may predate this socket or process. Restore the known
        // identity without treating its old contents as current live-edge evidence.
        crate::history::live::restore_receipt(session, message).await?;
        return Ok(true);
    }
    Ok(false)
}

pub(super) async fn mark(
    service: &OutgoingService,
    ids: Vec<super::Uuid>,
) -> Result<(), OutgoingError> {
    service
        .session
        .client
        .inner
        .registry
        .outgoing_projected(service.session.clone(), ids)
        .await
}

/// Receipt persistence is complete; projection failure cannot change its confirmed outcome.
/// A create enters history as a live insertion; an edit or deletion of `target` (the backend
/// ID the attempt resolved) is settled the way its socket event would settle it, so history
/// never shows the old text or the deleted message once the operation is Sent and projected.
pub(super) async fn finish(
    service: &OutgoingService,
    operation: &super::Operation,
    generation: crate::Generation,
    message: Option<crate::Message>,
    target: Option<super::Uuid>,
) -> Result<(), OutgoingError> {
    match operation.kind.as_ref() {
        super::OperationKind::Message(_) => {
            if service.session.inner.epochs.validate(generation).is_err() {
                return Ok(());
            }
            let message = message.ok_or(OutgoingError::InvalidData)?;
            crate::history::live::acknowledge(&service.session, generation, &message).await?;
        }
        super::OperationKind::Edit { .. } | super::OperationKind::Delete { .. } => {
            if service.session.inner.epochs.validate(generation).is_err() {
                return Ok(());
            }
            let target = target.ok_or(OutgoingError::InvalidData)?;
            // An edit receipt without `edited_at` changed nothing history could order.
            if let Some(event) = settlement(operation, target, message.as_ref()) {
                crate::history::live::settle(
                    &service.session,
                    generation,
                    operation.channel_id,
                    &event,
                )
                .await?;
            }
        }
        super::OperationKind::Reaction { .. } => {}
    }
    mark(service, vec![operation.id]).await
}

/// The socket event a confirmed edit or deletion of `target` amounts to: the edit's receipt
/// (its content and `edited_at`), or the deletion. None for a create or a reaction, and for an
/// edit whose receipt is missing or carries no `edited_at`.
fn settlement(
    operation: &super::Operation,
    target: super::Uuid,
    receipt: Option<&crate::Message>,
) -> Option<KnownEvent> {
    let channel_id = operation.channel_id.to_string();
    match operation.kind.as_ref() {
        super::OperationKind::Edit { .. } => {
            // Text only, as the socket's edit event carries: changed embeds arrive with their
            // own embeds update, and history never inserts an unheld message from an edit.
            let receipt = receipt?;
            Some(KnownEvent::MessageEdit(Box::new(MessageEditEvent {
                id: target.to_string(),
                channel_id,
                content: receipt.content.clone().unwrap_or_default(),
                edited_at: receipt.edited_at.clone()?,
            })))
        }
        super::OperationKind::Delete { .. } => {
            Some(KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
                id: target.to_string(),
                channel_id,
            })))
        }
        super::OperationKind::Message(_) | super::OperationKind::Reaction { .. } => None,
    }
}
