//! Public personal operations prove acknowledgement ordering, account isolation and generation cancellation.

use papo_api::wire::{
    Notifications, UpdateAvatarRequest, UpdateChannelUserSettingRequest, UpdateUserRequest,
    UpdateUserSettingsRequest, UserConfig,
};
use papo_sdk::{
    ClientError,
    operations::{
        OperationError, RefreshStatus, profile::ProfileChange, settings::NotificationPolicyChange,
    },
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
        let user = first.account().user();
        assert_eq!(
            resources
                .users
                .get(user)
                .and_then(|summary| summary.nickname.as_deref()),
            Some("changed")
        );
        assert_eq!(
            resources
                .profiles
                .get(user)
                .and_then(|profile| profile.description.as_deref()),
            Some("profile description")
        );
        let receipt = first
            .operations()
            .update_avatar(UpdateAvatarRequest {
                avatar: "fixture-inline-bytes".into(),
                avatar_format: "png".into(),
            })
            .await?;
        assert_eq!(receipt.refresh, RefreshStatus::Updated);
        let resources = first.subscribe_resources().borrow().clone();
        let profile = resources.profiles.get(user).ok_or("missing profile")?;
        assert_eq!(profile.avatar_format.as_deref(), Some("png"));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_change_of_one_profile_field_keeps_the_others_the_server_holds() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, _, _) = setup(&f).await?;
        let user = first.account().user();
        first
            .operations()
            .update_profile(UpdateUserRequest {
                nickname: "Lia".into(),
                status: "on call".into(),
                description: "about me".into(),
                typing: Some("is writing…".into()),
            })
            .await?;
        let receipt = first
            .operations()
            .change_profile(ProfileChange {
                nickname: Some("Lia M.".into()),
                ..ProfileChange::default()
            })
            .await?;
        assert_eq!(receipt.refresh, RefreshStatus::Updated);
        let resources = first.subscribe_resources().borrow().clone();
        let own = resources.users.get(user).ok_or("missing user")?;
        assert_eq!(own.nickname.as_deref(), Some("Lia M."));
        assert_eq!(own.status_message.as_deref(), Some("on call"), "kept");
        assert_eq!(own.typing.as_deref(), Some("is writing…"), "kept");
        let profile = resources.profiles.get(user).ok_or("missing profile")?;
        assert_eq!(profile.description.as_deref(), Some("about me"), "kept");
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn two_profile_changes_at_once_both_land() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, _, _) = setup(&f).await?;
        let user = first.account().user();
        first
            .operations()
            .update_profile(UpdateUserRequest {
                nickname: "Lia".into(),
                status: "on call".into(),
                ..Default::default()
            })
            .await?;
        let operations = first.operations();
        let (nickname, status) = tokio::join!(
            operations.change_profile(ProfileChange {
                nickname: Some("Lia M.".into()),
                ..ProfileChange::default()
            }),
            operations.change_profile(ProfileChange {
                status_message: Some("away for lunch".into()),
                ..ProfileChange::default()
            }),
        );
        nickname?;
        status?;
        let resources = first.subscribe_resources().borrow().clone();
        let own = resources.users.get(user).ok_or("missing user")?;
        assert_eq!(own.nickname.as_deref(), Some("Lia M."));
        assert_eq!(own.status_message.as_deref(), Some("away for lunch"));
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
        assert!(resources.profiles.get(user).is_some());
        assert_eq!(
            resources
                .users
                .get(user)
                .and_then(|summary| summary.nickname.as_deref()),
            Some("confirmed")
        );
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
                .get(channel)
                .and_then(|c| c.notification_settings.as_ref()),
            Some(&papo_sdk::BackendMode::Off)
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

#[test]
fn change_notification_settings_merges_one_field_and_preserves_theme_and_display() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, _, _) = setup(&f).await?;
        let receipt = first
            .operations()
            .change_notification_settings(NotificationPolicyChange {
                message_preview: Some(false),
                ..NotificationPolicyChange::default()
            })
            .await?;
        assert_eq!(receipt.refresh, RefreshStatus::Updated);
        let config = receipt.value.config.ok_or("missing config")?;
        assert_eq!(config.theme.as_deref(), Some("system"));
        let display = config.display.ok_or("missing display")?;
        assert_eq!(display.font_size.as_deref(), Some("medium"));
        assert_eq!(display.message_density.as_deref(), Some("normal"));
        assert_eq!(display.show_timestamps, Some(true));
        assert_eq!(display.show_avatars, Some(true));
        let notifications = config.notifications.ok_or("missing notifications")?;
        assert_eq!(notifications.enabled, Some(true));
        assert_eq!(notifications.mentions, Some(true));
        assert_eq!(notifications.message_preview, Some(false));
        assert_eq!(notifications.sound, Some(true));

        let policy = first
            .notifications()
            .account_policy()
            .ok_or("missing account policy")?;
        assert!(policy.enabled);
        assert!(policy.mentions);
        assert!(!policy.message_preview);
        assert!(policy.sound);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn two_notification_setting_changes_at_once_both_land() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, _, _) = setup(&f).await?;
        let operations = first.operations();
        let (mentions, sound) = tokio::join!(
            operations.change_notification_settings(NotificationPolicyChange {
                mentions: Some(false),
                ..NotificationPolicyChange::default()
            }),
            operations.change_notification_settings(NotificationPolicyChange {
                sound: Some(false),
                ..NotificationPolicyChange::default()
            }),
        );
        mentions?;
        sound?;
        let policy = first
            .notifications()
            .account_policy()
            .ok_or("missing account policy")?;
        assert!(policy.enabled);
        assert!(!policy.mentions);
        assert!(policy.message_preview);
        assert!(!policy.sound);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn change_notification_settings_refuses_incomplete_whoami_without_writing() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, _, _) = setup(&f).await?;
        let _ = f.server().take_operation_requests().await;
        f.server()
            .queue_operation_response(
                "GET",
                "/auth/whoami",
                200,
                serde_json::json!({
                    "id": first.account().user().to_string(),
                    "username": "op-first",
                    "settings": {"version": 1, "config": {"notifications": {"enabled": true}}}
                }),
            )
            .await?;
        let result = first
            .operations()
            .change_notification_settings(NotificationPolicyChange {
                sound: Some(false),
                ..NotificationPolicyChange::default()
            })
            .await;
        assert!(matches!(result, Err(OperationError::InvalidData)));
        let requests = f.server().take_operation_requests().await;
        assert!(
            !requests
                .iter()
                .any(|(method, path, _, _)| method == "PUT" && path == "/users/settings"),
            "incomplete whoami must not dispatch PUT /users/settings"
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
