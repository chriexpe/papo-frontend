//! Production self-profile/settings/social behavior and DM moderation restrictions.

use super::{Result, is_problem, wait_sent};
use papo_api::wire::{
    UpdateChannelUserSettingRequest, UpdateUserRequest, UpdateUserSettingsRequest, UserConfig,
};
use papo_sdk::{Session, Uuid};

pub async fn prove(member: &Session, other: &Session, owner: &Session, channel: Uuid) -> Result {
    let operations = member.operations();
    operations
        .update_profile(UpdateUserRequest {
            nickname: "synthetic nickname".into(),
            status: "synthetic status".into(),
            description: "synthetic bio".into(),
            typing: Some("synthetic typing".into()),
        })
        .await?;
    let settings = operations
        .update_user_settings(UpdateUserSettingsRequest {
            config: UserConfig {
                theme: Some("dark".into()),
                display: Some(papo_api::wire::Display {
                    font_size: Some("medium".into()),
                    message_density: Some("normal".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        })
        .await?
        .value;
    assert_eq!(
        settings.config.and_then(|value| value.theme).as_deref(),
        Some("dark")
    );
    operations
        .update_channel_settings(
            channel,
            UpdateChannelUserSettingRequest {
                notification_settings: "only_mentions".into(),
            },
        )
        .await?;
    let conversation = operations
        .open_direct_conversation(other.account().user())
        .await?
        .value;
    let dm: Uuid = conversation.id.parse()?;
    let setting = operations
        .update_channel_settings(
            dm,
            UpdateChannelUserSettingRequest {
                notification_settings: "off".into(),
            },
        )
        .await?;
    assert_eq!(
        setting.refresh,
        papo_sdk::RefreshStatus::Warning(papo_sdk::RefreshFault::Unavailable)
    );
    let queue = member.outgoing();
    let local = Uuid::from_u128(13001);
    queue
        .enqueue(
            dm,
            local,
            papo_sdk::OperationKind::Message(papo_sdk::OutgoingMessage {
                content: "synthetic direct message".into(),
                ..Default::default()
            }),
            papo_sdk::UnixMicros(0),
        )
        .await?;
    let message = wait_sent(member, local)
        .await?
        .message_id
        .ok_or("confirmed direct message identity")?;
    assert!(is_problem(
        &owner
            .operations()
            .moderate_delete_message(dm, message)
            .await,
        403,
        &papo_api::ProblemKind::Forbidden,
    ));
    operations.block_user(other.account().user()).await?;
    assert!(is_problem(
        &other
            .operations()
            .open_direct_conversation(member.account().user())
            .await,
        403,
        &papo_api::ProblemKind::DirectMessageBlocked,
    ));
    operations.unblock_user(other.account().user()).await?;
    other
        .operations()
        .open_direct_conversation(member.account().user())
        .await?;
    operations.close_direct_conversation(dm).await?;
    Ok(())
}
