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
    let generation = session.inner.epochs.current().generation;
    for op in pending {
        let Some(id) = op.message_id else {
            continue;
        };
        if let Some(event) = settlement(&op, id, op.message.as_deref()) {
            // An edit or deletion already confirmed is settled again; it is idempotent.
            crate::history::live::settle(session, generation, op.channel_id, &event).await?;
            ids.push(op.id);
        } else if crate::history::live::contains_receipt(session, op.channel_id, id).await {
            ids.push(op.id);
        } else if let Some(message) = &op.message {
            // Persisted receipts may predate this socket or process. Restore the known
            // identity without treating its old contents as current live-edge evidence.
            crate::history::live::restore_receipt(session, message).await?;
            ids.push(op.id);
        }
    }
    if !ids.is_empty() {
        mark(service, ids).await?;
        service.publish().await?;
    }
    Ok(())
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
/// A create enters history as a live insertion; an edit or deletion is settled the way its
/// socket event would settle it, so history never shows the old text or the deleted message
/// after the operation is Sent and projected.
pub(super) async fn finish(
    service: &OutgoingService,
    operation: &super::Operation,
    generation: crate::Generation,
    message: Option<crate::Message>,
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
            let target = operation.message_id.ok_or(OutgoingError::InvalidData)?;
            let event = settlement(operation, target, message.as_ref())
                .ok_or(OutgoingError::InvalidData)?;
            crate::history::live::settle(
                &service.session,
                generation,
                operation.channel_id,
                &event,
            )
            .await?;
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
