//! Retries local confirmed-receipt projection; never dispatches or infers sends from content.

use super::{OperationState, OutgoingError, OutgoingService};
use papo_api::gateway::{
    KnownEvent, MessageDeleteEvent, MessageEditEvent, MessageEmbedsUpdateEvent,
};

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
/// reaches the same state), or found in history when its receipt could not settle it; a create
/// is found in or restored to history. False when history does not hold it yet.
async fn project(
    session: &crate::Session,
    generation: crate::Generation,
    op: &super::Operation,
    id: super::Uuid,
) -> Result<bool, crate::ClientError> {
    match settlement(op, id, op.message.as_deref()) {
        Settlement::Events(events) => {
            for event in &events {
                crate::history::live::settle(session, generation, op.channel_id, event).await?;
            }
            return Ok(true);
        }
        Settlement::AwaitHistory { content } => {
            return Ok(
                crate::history::live::holds_edit(session, op.channel_id, id, &content).await,
            );
        }
        Settlement::NotAChange => {}
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
/// A reaction settles this account's membership on `target`, never its totals.
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
            match settlement(operation, target, message.as_ref()) {
                Settlement::Events(events) => {
                    for event in &events {
                        crate::history::live::settle(
                            &service.session,
                            generation,
                            operation.channel_id,
                            event,
                        )
                        .await?;
                    }
                }
                // Confirmed, but its receipt cannot say what history should hold: it stays
                // unprojected until history shows the edit (`reconcile_receipts`).
                Settlement::AwaitHistory { .. } => return Ok(()),
                Settlement::NotAChange => {}
            }
        }
        super::OperationKind::Reaction {
            emoji_id,
            unicode,
            add,
            ..
        } => {
            if service.session.inner.epochs.validate(generation).is_err() {
                return Ok(());
            }
            // Settled once, at confirmation: a replayed receipt carries no evidence that it is
            // newer than a page fetched since, so `reconcile_receipts` never settles it again.
            // A settle that fails, or a connection replaced meanwhile, leaves this account's
            // membership as the last page said until a page fetched after it arrives.
            let target = target.ok_or(OutgoingError::InvalidData)?;
            let change = crate::history::own_reactions::OwnReaction {
                emoji_id: emoji_id.map(|id| id.to_string()),
                unicode: unicode.clone(),
                add: *add,
                revision: 0,
            };
            crate::history::live::settle_own_reaction(
                &service.session,
                generation,
                operation.channel_id,
                target,
                change,
            )
            .await?;
        }
    }
    mark(service, vec![operation.id]).await
}

/// What settling a confirmed operation takes.
enum Settlement {
    /// The socket events it amounts to, applied in order.
    Events(Vec<KnownEvent>),
    /// An edit whose receipt lacks what history needs (its `content` or `edited_at`, or its
    /// embeds when it changed them): nothing is made up for it; it is settled once history
    /// holds an edit of the message to `content`, the text the operation asked for.
    AwaitHistory {
        /// The edit's requested text.
        content: String,
    },
    /// A create or a reaction: nothing an event would settle (a reaction's own membership is
    /// settled once, at confirmation, in `finish`).
    NotAChange,
}

/// What settling a confirmed edit or deletion of `target` takes. An edit's receipt settles its
/// text by its `edited_at`, as the socket's edit event would, and, when the edit replaced the
/// embeds, the receipt's embeds as the socket's embeds update would; history never inserts an
/// unheld message from either.
fn settlement(
    operation: &super::Operation,
    target: super::Uuid,
    receipt: Option<&crate::Message>,
) -> Settlement {
    let channel_id = operation.channel_id.to_string();
    match operation.kind.as_ref() {
        super::OperationKind::Edit {
            content, embeds, ..
        } => {
            let complete = receipt.and_then(|receipt| {
                let edit = KnownEvent::MessageEdit(Box::new(MessageEditEvent {
                    id: target.to_string(),
                    channel_id: channel_id.clone(),
                    content: receipt.content.clone()?,
                    edited_at: receipt.edited_at.clone()?,
                }));
                let mut events = vec![edit];
                if embeds.is_some() {
                    events.push(KnownEvent::MessageEmbedsUpdate(Box::new(
                        MessageEmbedsUpdateEvent {
                            channel_id: channel_id.clone(),
                            message_id: target.to_string(),
                            embeds: receipt.embeds.clone()?,
                        },
                    )));
                }
                Some(events)
            });
            complete.map_or_else(
                || Settlement::AwaitHistory {
                    content: content.clone(),
                },
                Settlement::Events,
            )
        }
        super::OperationKind::Delete { .. } => Settlement::Events(vec![KnownEvent::MessageDelete(
            Box::new(MessageDeleteEvent {
                id: target.to_string(),
                channel_id,
            }),
        )]),
        super::OperationKind::Message(_) | super::OperationKind::Reaction { .. } => {
            Settlement::NotAChange
        }
    }
}
