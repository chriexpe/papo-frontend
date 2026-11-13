//! Retries local confirmed-receipt projection; never dispatches or infers sends from content.

use super::{OperationState, OutgoingError, OutgoingService};

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
    for op in pending {
        let Some(id) = op.message_id else {
            continue;
        };
        if crate::history::live::contains_receipt(session, op.channel_id, id).await {
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
pub(super) async fn finish(
    service: &OutgoingService,
    operation: &super::Operation,
    generation: crate::Generation,
    message: Option<crate::Message>,
) -> Result<(), OutgoingError> {
    if matches!(operation.kind.as_ref(), super::OperationKind::Message(_)) {
        if service.session.inner.epochs.validate(generation).is_err() {
            return Ok(());
        }
        let message = message.ok_or(OutgoingError::InvalidData)?;
        crate::history::live::acknowledge(&service.session, generation, &message).await?;
    }
    mark(service, vec![operation.id]).await
}
