//! Real scoped webhook identity/embeds and backend image-format authorization.

use super::{PNG, Result, id, is_problem};
use papo_api::wire::{
    CreateEmojiRequest, CreateWebhookRequest, EmbedInput, ExternalAuthor,
    UpdateWebhookIdentityRequest,
};
use papo_sdk::{OutgoingMessage, Session, Uuid, operations::webhooks::WebhookMessage};

pub async fn prove(owner: &Session, member: &Session, channel: Uuid) -> Result {
    let operations = owner.operations();
    let created = operations
        .create_webhook(CreateWebhookRequest {
            name: "sdk-proof-hook".into(),
            channel_id: channel.to_string(),
        })
        .await?
        .value;
    let webhook = id(created.webhook.id.as_deref())?;
    let foreign = operations
        .execute_webhook(
            webhook,
            created.token.clone(),
            WebhookMessage {
                message: OutgoingMessage {
                    content: "synthetic foreign message".into(),
                    embeds: vec![EmbedInput {
                        title: Some("synthetic confirmed embed".into()),
                        ..Default::default()
                    }],
                    ..Default::default()
                },
                external_author: Some(ExternalAuthor {
                    source: Some("synthetic-provider".into()),
                    id: Some("synthetic-foreign-id".into()),
                    name: Some("synthetic foreign author".into()),
                    ..Default::default()
                }),
                external_message_id: Some("sdk-proof-external-id".into()),
            },
        )
        .await?
        .value;
    assert_eq!(
        foreign.webhook_id.as_deref(),
        Some(webhook.to_string().as_str())
    );
    let author = foreign.external_author.as_ref().ok_or("foreign identity")?;
    assert_eq!(author.id.as_deref(), Some("synthetic-foreign-id"));
    assert!(foreign.embeds.as_ref().is_some_and(|embeds| {
        embeds
            .iter()
            .any(|embed| embed.title.as_deref() == Some("synthetic confirmed embed"))
    }));
    let message = id(foreign.id.as_deref())?;
    operations
        .update_webhook(
            webhook,
            UpdateWebhookIdentityRequest {
                display_name: Some("synthetic identity".into()),
                ..Default::default()
            },
        )
        .await?;
    let edited = operations
        .edit_webhook_message(
            webhook,
            message,
            created.token.clone(),
            "synthetic edited foreign".into(),
            None,
        )
        .await?
        .value;
    assert_eq!(
        edited
            .external_author
            .as_ref()
            .and_then(|author| author.id.as_deref()),
        Some("synthetic-foreign-id")
    );
    super::interactions::prove(owner, member, channel, message).await?;
    prove_revoke_and_denial(owner, member, channel, webhook, message, created.token).await?;
    prove_emoji(owner, member).await
}

async fn prove_revoke_and_denial(
    owner: &Session,
    member: &Session,
    channel: Uuid,
    webhook: Uuid,
    message: Uuid,
    token: String,
) -> Result {
    let operations = owner.operations();
    assert!(is_problem(
        &member
            .operations()
            .create_webhook(CreateWebhookRequest {
                name: "denied".into(),
                channel_id: channel.to_string(),
            })
            .await,
        403,
        &papo_api::ProblemKind::Forbidden,
    ));
    operations
        .delete_webhook_message(webhook, message, token.clone())
        .await?;
    operations.delete_webhook(webhook).await?;
    assert!(is_problem(
        &operations
            .execute_webhook(
                webhook,
                token,
                WebhookMessage {
                    message: OutgoingMessage {
                        content: "revoked".into(),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )
            .await,
        401,
        &papo_api::ProblemKind::Unauthorized,
    ));
    Ok(())
}

async fn prove_emoji(owner: &Session, member: &Session) -> Result {
    // A complete synthetic 1x1 PNG; no external media, credentials or client decoders.
    let request = CreateEmojiRequest {
        name: "sdk_proof_static".into(),
        format: "PNG".into(),
        image_blob: PNG.into(),
    };
    assert!(is_problem(
        &member.operations().create_emoji(request.clone()).await,
        403,
        &papo_api::ProblemKind::Forbidden,
    ));
    let emoji = owner.operations().create_emoji(request).await?.value;
    owner
        .operations()
        .delete_emoji(id(emoji.id.as_deref())?)
        .await?;
    Ok(())
}
