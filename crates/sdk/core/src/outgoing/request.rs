//! Wire request execution and receipt validation for one claimed operation; queue transitions stay in dispatch.

use super::{Operation, OperationKind, Uuid, persistence::Outcome};
use papo_api::{
    MessageAttachment, MessageUpload, RestClient,
    wire::{ReactionRequest, UpdateMessageRequest},
};

pub(super) async fn send(
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
