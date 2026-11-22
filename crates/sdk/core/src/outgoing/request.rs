//! Wire request execution and receipt validation for one claimed operation; queue transitions stay in dispatch.

use super::{
    Operation, OperationKind, Uuid,
    persistence::Outcome,
    progress::ProgressReporter,
    stream::{OpenedAttachment, StagedFileStream},
};
use crate::Session;
use papo_api::{
    RestClient, StreamedMessageAttachment, StreamedMessageUpload,
    wire::{ReactionRequest, UpdateMessageRequest},
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(super) async fn send(
    session: &Session,
    epoch_cancel: &CancellationToken,
    rest: &RestClient,
    operation: &Operation,
    opened: Vec<OpenedAttachment>,
    progress: Option<Arc<ProgressReporter>>,
    target: Option<Uuid>,
) -> Result<Outcome, papo_api::Error> {
    let author = session.account().user();
    let message = match operation.kind.as_ref() {
        OperationKind::Message(message) => {
            if opened.len() != message.attachments.len() {
                return Err(papo_api::Error::NotDispatched);
            }
            let attachments = build_streams(session, epoch_cancel, opened, progress)?;
            let embeds =
                serde_json::to_string(&message.embeds).map_err(|_| papo_api::Error::Decode)?;
            let reply = target.map(|id| id.to_string());
            Some(
                rest.create_message_streamed(StreamedMessageUpload {
                    channel_id: &operation.channel_id.to_string(),
                    content: Some(&message.content),
                    reply_to: reply.as_deref(),
                    embeds: (!message.embeds.is_empty()).then_some(embeds.as_str()),
                    attachments,
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
            match rest
                .delete_message(target.ok_or(papo_api::Error::Decode)?)
                .await
            {
                // Already gone (another deletion got there first): what was asked for holds.
                Ok(())
                | Err(
                    papo_api::Error::Problem { status: 404, .. }
                    | papo_api::Error::Http { status: 404 },
                ) => {}
                Err(error) => return Err(error),
            }
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
        // A deletion is projected once history has settled it; a reaction has nothing to
        // settle, its totals stay the backend's.
        let deletion = matches!(operation.kind.as_ref(), OperationKind::Delete { .. });
        Ok(Outcome {
            projected: !deletion,
            message_id: target,
            message: None,
        })
    }
}

fn build_streams(
    session: &Session,
    epoch_cancel: &CancellationToken,
    opened: Vec<OpenedAttachment>,
    progress: Option<Arc<ProgressReporter>>,
) -> Result<Vec<StreamedMessageAttachment>, papo_api::Error> {
    if opened.is_empty() {
        return Ok(Vec::new());
    }
    let progress = progress.ok_or(papo_api::Error::NotDispatched)?;
    let lease = Arc::clone(&session.client.inner.registry.lease);
    let mut attachments = Vec::with_capacity(opened.len());
    for item in opened {
        let size_bytes = item.reference.size_bytes;
        let stream = StagedFileStream::new(
            session,
            item.file,
            item.reference,
            epoch_cancel.clone(),
            Arc::clone(&lease),
            Arc::clone(&progress),
        );
        attachments.push(StreamedMessageAttachment {
            filename: item.filename,
            size_bytes,
            stream: Box::pin(stream),
        });
    }
    Ok(attachments)
}
