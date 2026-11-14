//! Public personal operations prove acknowledgement ordering, account isolation and generation cancellation.

use papo_api::wire::{
    Notifications, UpdateAvatarRequest, UpdateChannelUserSettingRequest, UpdateUserRequest,
    UpdateUserSettingsRequest, UserConfig,
};
use papo_sdk::{
    ClientError,
    operations::{OperationError, RefreshStatus},
};

use super::{Fixture, Result, setup};

#[test]
fn acknowledged_profile_refresh_retains_metadata_without_inline_avatar_bytes() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, _) = setup(&f).await?;
        let receipt = first
            .operations()
            .update_profile(UpdateUserRequest {
                nickname: "changed".into(),
                status: "busy working".into(),
                description: "profile description".into(),
                typing: Some("typing phrase".into()),
            })
            .await?;
        assert_eq!(receipt.refresh, RefreshStatus::Updated);
        receipt.validate(&first)?;
        assert!(receipt.validate(&second).is_err());
        let resources = first.subscribe_resources().borrow().clone();
        let profile = resources
            .profiles
            .iter()
            .find(|profile| profile.id.as_deref() == Some(&first.account().user().to_string()))
            .ok_or("missing profile")?;
        assert_eq!(profile.nickname.as_deref(), Some("changed"));
        assert_eq!(profile.status_message.as_deref(), Some("busy working"));
        let receipt = first
            .operations()
            .update_avatar(UpdateAvatarRequest {
                avatar: "fixture-inline-bytes".into(),
                avatar_format: "png".into(),
            })
            .await?;
        assert_eq!(receipt.refresh, RefreshStatus::Updated);
        let resources = first.subscribe_resources().borrow().clone();
        let profile = resources
            .profiles
            .iter()
            .find(|profile| profile.id.as_deref() == Some(&first.account().user().to_string()))
            .ok_or("missing profile")?;
        assert!(profile.avatar_blob.is_none());
        assert_eq!(profile.avatar_format.as_deref(), Some("png"));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn rejected_profile_mutation_preserves_last_confirmed_projection() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, _, _) = setup(&f).await?;
        let user = first.account().user();
        first
            .operations()
            .update_profile(UpdateUserRequest {
                nickname: "confirmed".into(),
                ..Default::default()
            })
            .await?;
        f.server()
            .fail_next_rest("PUT", &format!("/users/{user}"), 403)
            .await?;
        let result = first
            .operations()
            .update_profile(UpdateUserRequest {
                nickname: "denied".into(),
                ..Default::default()
            })
            .await;
        assert!(matches!(
            result,
            Err(OperationError::Session(ClientError::Api(
                papo_api::Error::Problem { status: 403, .. }
            )))
        ));
        let resources = first.subscribe_resources().borrow().clone();
        let profile = resources
            .profiles
            .iter()
            .find(|profile| profile.id.as_deref() == Some(&user.to_string()))
            .ok_or("missing profile")?;
        assert_eq!(profile.nickname.as_deref(), Some("confirmed"));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn account_and_channel_settings_refresh_without_changing_another_account() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let receipt = first
            .operations()
            .update_user_settings(UpdateUserSettingsRequest {
                config: UserConfig {
                    notifications: Some(Notifications {
                        enabled: Some(false),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            })
            .await?;
        assert_eq!(receipt.account, first.account());
        assert_eq!(
            receipt
                .value
                .config
                .and_then(|c| c.notifications)
                .and_then(|n| n.enabled),
            Some(false)
        );
        assert_eq!(receipt.refresh, RefreshStatus::Updated);
        let before = second.subscribe_resources().borrow().clone();
        let receipt = first
            .operations()
            .update_channel_settings(
                channel,
                UpdateChannelUserSettingRequest {
                    notification_settings: "off".into(),
                },
            )
            .await?;
        assert_eq!(receipt.refresh, RefreshStatus::Updated);
        assert_eq!(
            receipt.value.user_id.as_deref(),
            Some(first.account().user().to_string().as_str())
        );
        let resources = first.subscribe_resources().borrow().clone();
        assert_eq!(
            resources
                .channels
                .iter()
                .find(|c| c.id.as_deref() == Some(channel.to_string().as_str()))
                .and_then(|c| c.notification_settings.as_deref()),
            Some("off")
        );
        assert_eq!(
            second.subscribe_resources().borrow().version,
            before.version
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn direct_conversation_and_block_actions_refresh_only_the_requesting_account() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, _) = setup(&f).await?;
        let receipt = first
            .operations()
            .open_direct_conversation(second.account().user())
            .await?;
        let id = receipt.value.id.parse()?;
        assert_eq!(receipt.refresh, RefreshStatus::Updated);
        assert_eq!(first.subscribe_resources().borrow().dms.len(), 1);
        assert!(second.subscribe_resources().borrow().dms.is_empty());
        assert_eq!(
            first
                .operations()
                .direct_conversation(id)
                .await?
                .value
                .user
                .id
                .as_deref(),
            Some(second.account().user().to_string().as_str())
        );
        first
            .operations()
            .block_user(second.account().user())
            .await?;
        assert_eq!(
            first.operations().blocked_users().await?.value.users.len(),
            1
        );
        first
            .operations()
            .unblock_user(second.account().user())
            .await?;
        assert!(first.subscribe_resources().borrow().blocks.is_empty());
        first.operations().close_direct_conversation(id).await?;
        assert!(first.subscribe_resources().borrow().dms.is_empty());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn reconnect_cancels_a_held_personal_read_without_publishing_its_old_generation() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, _, _) = setup(&f).await?;
        let generation = first.generation();
        let mut hold = f.server().hold_next_rest("GET", "/dms").await?;
        let operations = first.operations();
        let pending = tokio::spawn(async move { operations.direct_conversations().await });
        hold.wait_started().await?;
        first.reconnect()?;
        let result = pending.await?;
        assert!(matches!(
            result,
            Err(OperationError::Session(ClientError::Superseded))
        ));
        assert_ne!(first.generation(), generation);
        hold.release();
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn accepted_dm_setting_does_not_claim_unavailable_backend_policy_readback() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, _) = setup(&f).await?;
        let conversation = first
            .operations()
            .open_direct_conversation(second.account().user())
            .await?;
        let channel = conversation.value.id.parse()?;
        let receipt = first
            .operations()
            .update_channel_settings(
                channel,
                UpdateChannelUserSettingRequest {
                    notification_settings: "off".into(),
                },
            )
            .await?;
        assert_eq!(
            receipt.refresh,
            RefreshStatus::Warning(papo_sdk::RefreshFault::Unavailable)
        );
        assert_eq!(receipt.value.notification_settings.as_deref(), Some("off"));
        receipt.validate(&first)?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
