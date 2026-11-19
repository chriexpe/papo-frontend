//! Loopback regressions for quarantined deferral, denied-channel suppression and background
//! wire resource admission.

use super::{
    read_evidence::{DurableKind, ledger_rows, seed_read_points, timestamp_offset},
    *,
};
use papo_api::wire::{
    ChannelPermissionEntry, ChannelPermissions, DirectConversation, Role, RolePermissions,
    UserSummary,
};
use papo_sdk::{
    CacheResource, HistoryError, MessageKey, NotificationStatus, operations::MetadataScope,
};
use serde_json::json;

async fn quarantine_channel(f: &Fixture, session: &Session, peer: Uuid, channel: Uuid) -> Result {
    // Listing the same UUID as both a channel and a DM yields `ReadAccess::Unknown`
    // (`Admission::Quarantined`) without modifying any permission algorithm.
    f.server()
        .set_direct_conversations(
            &session.account().user().to_string(),
            vec![DirectConversation {
                id: channel.to_string(),
                user: UserSummary {
                    id: Some(peer.to_string()),
                    username: Some("peer".into()),
                    ..Default::default()
                },
                ..Default::default()
            }],
        )
        .await?;
    let mut resources = session.subscribe_resources();
    while !resources
        .borrow()
        .dms
        .iter()
        .any(|dm| dm.id == channel.to_string())
    {
        resources.changed().await?;
    }
    assert!(matches!(
        session.history(channel).await,
        Err(HistoryError::AccessUnknown)
    ));
    Ok(())
}

async fn release_quarantine(f: &Fixture, session: &Session, channel: Uuid) -> Result {
    f.server()
        .set_direct_conversations(&session.account().user().to_string(), Vec::new())
        .await?;
    session
        .operations()
        .refresh_metadata(MetadataScope::Social)
        .await?;
    assert!(session.history(channel).await.is_ok());
    Ok(())
}

#[test]
fn quarantined_channel_defers_before_claim_across_background_push_and_live() -> Result {
    for already_read in [false, true] {
        let f = Fixture::new(papo_test_server::Config::default())?;
        f.run(async {
            let (first, second, channel) = setup(&f).await?;
            let client = f.client().handle();
            client.push_wake(first.account(), deadline()).await?;
            quarantine_channel(&f, &first, second.account().user(), channel).await?;

            let bg_at = timestamp_offset(-2);
            let live_at = timestamp_offset(0);
            let bg_row = row(
                1,
                channel,
                second.account().user(),
                first.account().user(),
                &bg_at,
            );
            let live_id = Uuid::from_u128(10_099);
            if already_read {
                let boundary =
                    MessageKey::parse(&timestamp_offset(5), &Uuid::from_u128(90_000).to_string())?;
                seed_read_points(&f, &first, channel, DurableKind::Local, boundary, live_id)?;
            }
            f.server()
                .set_notifications(first.account().user(), vec![bg_row])
                .await?;

            // Background and push both see NotReady -> More, 0 deliveries, 0 ledger claims.
            let bg = client.background_sync(deadline()).await?;
            assert_eq!(
                bg.accounts
                    .iter()
                    .find(|a| a.account == first.account())
                    .ok_or("first")?
                    .outcome,
                BackgroundOutcome::More
            );
            assert!(bg.deliveries.is_empty());
            let push = client.push_wake(first.account(), deadline()).await?;
            assert_eq!(push.accounts[0].outcome, BackgroundOutcome::More);
            assert!(push.deliveries.is_empty());
            assert_eq!(ledger_rows(&f)?, 0);

            // Live mention also defers before claim without faulting.
            let live_msg = MessageEvent {
                id: live_id.to_string(),
                channel_id: channel.to_string(),
                author_id: second.account().user().to_string(),
                content: format!("@mention(<@{}>)", first.account().user()),
                created_at: live_at,
                ..Default::default()
            };
            f.server()
                .emit_user_event(
                    &first.account().user().to_string(),
                    KnownEvent::Message(Box::new(live_msg)),
                )
                .await?;
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(120),
                    first.notifications().next_delivery(),
                )
                .await
                .is_err()
            );
            assert_eq!(
                *first.notifications().subscribe_status().borrow(),
                NotificationStatus::Ready
            );
            assert_eq!(ledger_rows(&f)?, 0);

            // Releasing quarantine lets live wake automatically and background finish cleanly.
            release_quarantine(&f, &first, channel).await?;
            if already_read {
                assert!(
                    tokio::time::timeout(
                        Duration::from_millis(150),
                        first.notifications().next_delivery(),
                    )
                    .await
                    .is_err()
                );
            } else {
                let delivered = tokio::time::timeout(
                    Duration::from_secs(5),
                    first.notifications().next_delivery(),
                )
                .await??;
                assert_eq!(delivered.target.message, live_id);
            }
            let after = client.push_wake(first.account(), deadline()).await?;
            assert_eq!(after.accounts[0].outcome, BackgroundOutcome::Complete);
            assert_eq!(after.deliveries.len(), usize::from(!already_read));
            // Both candidates (live + background) now have durable ledger claims.
            loop {
                if ledger_rows(&f)? == 2 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Ok::<_, Box<dyn std::error::Error>>(())
        })??;
        f.finish()?;
    }
    Ok(())
}

async fn reconnect_online(session: &Session) -> Result {
    let before_gen = session.generation().get();
    session.reconnect()?;
    let mut status = session.subscribe_connection();
    while !(status.borrow().state == papo_sdk::ConnectionState::Online
        && status.borrow().generation.get() > before_gen)
    {
        status.changed().await?;
    }
    Ok(())
}

async fn create_denied_channel(f: &Fixture) -> Result<Uuid> {
    let denied: Uuid = f
        .server()
        .create_channel("denied")
        .await?
        .id
        .ok_or("channel")?
        .parse()?;
    let role = Uuid::from_u128(0x3000_0001).to_string();
    f.server()
        .set_roles(vec![Role {
            id: Some(role.clone()),
            permissions: Some(RolePermissions::default()),
            ..Role::default()
        }])
        .await;
    let mut metadata = f.server().create_channel("placeholder").await?;
    metadata.id = Some(denied.to_string());
    metadata.name = Some("restricted".into());
    metadata.permissions = Some(vec![ChannelPermissionEntry {
        role_id: Some(role),
        permissions: Some(ChannelPermissions {
            read_channel: Some(false),
            ..Default::default()
        }),
        ..Default::default()
    }]);
    f.server().set_channel(metadata).await?;
    Ok(denied)
}

#[test]
fn denied_channel_suppresses_notifications_without_stalling_healthy_channels() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        // `first` is server owner; use `second` (non-owner) so channel overwrites deny read access.
        let (owner, member, healthy) = setup(&f).await?;
        let denied = create_denied_channel(&f).await?;
        reconnect_online(&member).await?;
        assert!(matches!(
            member.history(denied).await,
            Err(HistoryError::Closed(_))
        ));
        let denied_target = |generation| papo_sdk::NotificationTarget {
            account: member.account(),
            generation,
            channel: denied,
            message: Uuid::from_u128(88_001),
        };
        assert!(
            member
                .notifications()
                .validate_target(&denied_target(member.generation()))
                .is_err()
        );
        // A second reconnect retains the durable denial fence on the new generation too.
        reconnect_online(&member).await?;
        assert!(
            member
                .notifications()
                .validate_target(&denied_target(member.generation()))
                .is_err()
        );

        let client = f.client().handle();
        client.push_wake(member.account(), deadline()).await?;
        let at = timestamp();
        // Live mention on denied channel must not deliver.
        f.server()
            .emit_user_event(
                &member.account().user().to_string(),
                KnownEvent::Message(Box::new(MessageEvent {
                    id: Uuid::from_u128(88_001).to_string(),
                    channel_id: denied.to_string(),
                    author_id: owner.account().user().to_string(),
                    content: format!("@mention(<@{}>)", member.account().user()),
                    created_at: at.clone(),
                    ..Default::default()
                })),
            )
            .await?;
        assert!(
            tokio::time::timeout(
                Duration::from_millis(120),
                member.notifications().next_delivery(),
            )
            .await
            .is_err()
        );
        // Background sync suppresses denied channel and still delivers healthy channel.
        f.server()
            .set_notifications(
                member.account().user(),
                vec![
                    row(
                        1,
                        denied,
                        owner.account().user(),
                        member.account().user(),
                        &at,
                    ),
                    row(
                        2,
                        healthy,
                        owner.account().user(),
                        member.account().user(),
                        &at,
                    ),
                ],
            )
            .await?;
        let report = client.push_wake(member.account(), deadline()).await?;
        assert_eq!(report.accounts[0].outcome, BackgroundOutcome::Complete);
        assert_eq!(report.deliveries.len(), 1);
        assert_eq!(report.deliveries[0].target.channel, healthy);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

async fn emit_reply_pair(
    f: &Fixture,
    recipient: &Session,
    author: Uuid,
    channel: Uuid,
    reply_id: Uuid,
    at: &str,
) -> Result {
    let mut reply_row = row(1, channel, author, recipient.account().user(), at);
    reply_row.message_id = Some(reply_id.to_string());
    reply_row.message_content = Some(String::new());
    f.server()
        .set_notifications(recipient.account().user(), vec![reply_row.clone()])
        .await?;
    f.server()
        .emit_user_event(
            &recipient.account().user().to_string(),
            KnownEvent::Message(Box::new(MessageEvent {
                id: reply_id.to_string(),
                channel_id: channel.to_string(),
                author_id: author.to_string(),
                content: String::new(),
                created_at: at.into(),
                ..Default::default()
            })),
        )
        .await?;
    f.server()
        .emit_user_event(
            &recipient.account().user().to_string(),
            KnownEvent::NewNotification(Box::new(NewNotificationEvent {
                id: reply_row.id.ok_or("id")?,
                user_id: author.to_string(),
                message_id: reply_id.to_string(),
                message_content: String::new(),
            })),
        )
        .await?;
    Ok(())
}

#[test]
fn quarantined_live_reply_upgrades_and_unrelated_event_does_not_swallow_wakeup() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let healthy: Uuid = f
            .server()
            .create_channel("healthy")
            .await?
            .id
            .ok_or("channel")?
            .parse()?;
        let mut resources = first.subscribe_resources();
        while !resources
            .borrow()
            .channels
            .iter()
            .any(|c| c.id.as_deref().and_then(|id| id.parse().ok()) == Some(healthy))
        {
            resources.changed().await?;
        }
        quarantine_channel(&f, &first, second.account().user(), channel).await?;

        let quarantined_target = papo_sdk::NotificationTarget {
            account: first.account(),
            generation: first.generation(),
            channel,
            message: Uuid::from_u128(44_001),
        };
        assert!(
            first
                .notifications()
                .validate_target(&quarantined_target)
                .is_err()
        );

        let at = timestamp();
        let reply_id = Uuid::from_u128(44_001);
        emit_reply_pair(&f, &first, second.account().user(), channel, reply_id, &at).await?;
        assert!(
            tokio::time::timeout(
                Duration::from_millis(120),
                first.notifications().next_delivery(),
            )
            .await
            .is_err()
        );
        assert_eq!(ledger_rows(&f)?, 0);

        // Release quarantine AND emit an unrelated healthy-channel mention: neither the reply
        // upgrade nor the watch wakeup may be lost.
        release_quarantine(&f, &first, channel).await?;
        let healthy_id = Uuid::from_u128(44_002);
        f.server()
            .emit_user_event(
                &first.account().user().to_string(),
                KnownEvent::Message(Box::new(MessageEvent {
                    id: healthy_id.to_string(),
                    channel_id: healthy.to_string(),
                    author_id: second.account().user().to_string(),
                    content: format!("@mention(<@{}>)", first.account().user()),
                    created_at: at,
                    ..Default::default()
                })),
            )
            .await?;
        let mut delivered = std::collections::BTreeSet::new();
        for _ in 0..2 {
            let item = tokio::time::timeout(
                Duration::from_secs(5),
                first.notifications().next_delivery(),
            )
            .await??;
            delivered.insert(item.target.message);
        }
        assert_eq!(delivered, [reply_id, healthy_id].into());
        assert_eq!(
            *first.notifications().subscribe_status().borrow(),
            NotificationStatus::Ready
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn background_policy_resources_use_wire_admission_and_never_corrupt_session_resources() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, _, channel) = setup(&f).await?;
        let client = f.client().handle();
        let before_channels = first.subscribe_resources().borrow().channels.len();

        for (method, path, body, expected_group) in [
            ("GET", "/channels", json!({}), CacheResource::Channels),
            (
                "GET",
                "/channels",
                json!({ "channels": null }),
                CacheResource::Channels,
            ),
            (
                "GET",
                "/channels",
                json!({ "channels": [{ "id": channel.to_string(), "type": "text" }] }),
                CacheResource::Channels,
            ),
            (
                "GET",
                "/dms",
                json!({
                    "dms": [{
                        "id": "not-a-uuid",
                        "created_at": "2026-01-01T00:00:00Z",
                        "unread_count": 0,
                        "user": { "id": first.account().user().to_string() }
                    }]
                }),
                CacheResource::Dms,
            ),
        ] {
            let rejected_before = first.resource_admission().rejected_replacements;
            f.server()
                .queue_operation_response(method, path, 200, body)
                .await?;
            let report = client.push_wake(first.account(), deadline()).await?;
            assert_eq!(report.accounts[0].outcome, BackgroundOutcome::Failed);
            assert!(report.deliveries.is_empty());
            let diagnostics = first.resource_admission();
            assert_eq!(diagnostics.rejected_replacements, rejected_before + 1);
            assert_eq!(
                diagnostics.last_rejection.map(|rejection| rejection.group),
                Some(expected_group)
            );
            assert_eq!(
                first.subscribe_resources().borrow().channels.len(),
                before_channels
            );
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
