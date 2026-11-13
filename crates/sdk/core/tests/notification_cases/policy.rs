//! Producer policy edge cases and per-account fault isolation.
use super::*;

#[test]
fn partial_account_failure_and_inaccessible_row_do_not_block_healthy_progress() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let client = f.client().handle();
        let path = format!("/users/{}/notifications", first.account().user());
        f.server().fail_next_rest("GET", &path, 503).await?;
        let report = client.background_sync(deadline()).await?;
        assert_eq!(
            report
                .accounts
                .iter()
                .find(|r| r.account == first.account())
                .ok_or("first")?
                .outcome,
            BackgroundOutcome::Failed
        );
        assert_eq!(
            report
                .accounts
                .iter()
                .find(|r| r.account == second.account())
                .ok_or("second")?
                .outcome,
            BackgroundOutcome::Complete
        );
        client.background_sync(deadline()).await?;
        let denied: Uuid = f
            .server()
            .create_channel("denied")
            .await?
            .id
            .ok_or("channel")?
            .parse()?;
        f.server()
            .set_channel_readable(
                &first.account().user().to_string(),
                &denied.to_string(),
                false,
            )
            .await?;
        let at = timestamp();
        f.server()
            .set_notifications(
                first.account().user(),
                vec![
                    row(
                        1,
                        denied,
                        second.account().user(),
                        first.account().user(),
                        &at,
                    ),
                    row(
                        2,
                        channel,
                        second.account().user(),
                        first.account().user(),
                        &at,
                    ),
                ],
            )
            .await?;
        let report = client.push_wake(first.account(), deadline()).await?;
        assert_eq!(report.deliveries.len(), 1);
        assert_eq!(report.deliveries[0].target.channel, channel);
        assert!(
            client
                .push_wake(first.account(), deadline())
                .await?
                .deliveries
                .is_empty()
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn backend_all_does_not_turn_ephemeral_events_into_device_mention_alerts() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let mut metadata = first
            .subscribe_resources()
            .borrow()
            .channels
            .iter()
            .find(|c| c.id.as_deref().and_then(|id| id.parse().ok()) == Some(channel))
            .cloned()
            .ok_or("channel")?;
        metadata.notification_settings = Some("all".into());
        metadata.name = Some("changed".into());
        f.server().set_channel(metadata).await?;
        let mut resources = first.subscribe_resources();
        loop {
            if resources.borrow().channels.iter().any(|c| {
                c.id.as_deref().and_then(|id| id.parse().ok()) == Some(channel)
                    && c.notification_settings.as_deref() == Some("all")
            }) {
                break;
            }
            resources.changed().await?;
        }
        let mut settings = first.notifications().settings().await?;
        settings
            .channels
            .insert(channel, papo_sdk::ChannelNotificationMode::Mentions);
        first.notifications().set_settings(settings).await?;
        let at = timestamp();
        let mut record = row(
            1,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        record.message_content = Some(String::new());
        let message = MessageEvent {
            id: record.message_id.clone().ok_or("id")?,
            channel_id: channel.to_string(),
            author_id: second.account().user().to_string(),
            content: String::new(),
            created_at: at,
            ..Default::default()
        };
        let event = NewNotificationEvent {
            id: Uuid::from_u128(999).to_string(),
            user_id: second.account().user().to_string(),
            message_id: record.message_id.clone().ok_or("id")?,
            message_content: String::new(),
        };
        f.server()
            .emit_user_event(
                &first.account().user().to_string(),
                KnownEvent::Message(Box::new(message)),
            )
            .await?;
        f.server()
            .emit_user_event(
                &first.account().user().to_string(),
                KnownEvent::NewNotification(Box::new(event.clone())),
            )
            .await?;
        assert!(
            tokio::time::timeout(
                Duration::from_millis(100),
                first.notifications().next_delivery()
            )
            .await
            .is_err()
        );
        f.server()
            .set_notifications(first.account().user(), vec![record])
            .await?;
        f.server()
            .emit_user_event(
                &first.account().user().to_string(),
                KnownEvent::NewNotification(Box::new(event)),
            )
            .await?;
        let delivery = first.notifications().next_delivery().await?;
        assert_eq!(delivery.target.channel, channel);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn rejected_push_transfer_preserves_known_owner_and_server_failures_are_unknown() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, _) = setup(&f).await?;
        first
            .notifications()
            .register_push(device("synthetic-token"))
            .await?;
        f.server()
            .fail_next_rest("PUT", "/users/me/push-device", 403)
            .await?;
        assert!(
            second
                .notifications()
                .register_push(device("synthetic-token"))
                .await
                .is_err()
        );
        assert_eq!(
            first.notifications().unregister_push().await?,
            PushRegistration::Removed
        );
        assert!(f.server().push_owner("synthetic-token").await.is_none());
        f.server()
            .fail_next_rest("PUT", "/users/me/push-device", 503)
            .await?;
        assert_eq!(
            second
                .notifications()
                .register_push(device("synthetic-token"))
                .await?,
            PushRegistration::Unknown
        );
        assert_eq!(
            second
                .notifications()
                .register_push(device("synthetic-token"))
                .await?,
            PushRegistration::Unknown
        );
        assert_eq!(
            second
                .notifications()
                .unregister_push_device(device("synthetic-token"))
                .await?,
            PushRegistration::Removed
        );
        assert_eq!(
            second
                .notifications()
                .register_push(device("synthetic-token"))
                .await?,
            PushRegistration::Registered
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn a_claimed_delivery_cannot_escape_after_logout_and_same_account_replacement() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let at = timestamp();
        let record = row(
            1,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        let message = MessageEvent {
            id: record.message_id.clone().ok_or("id")?,
            channel_id: channel.to_string(),
            author_id: second.account().user().to_string(),
            content: record.message_content.clone().ok_or("content")?,
            created_at: at,
            ..Default::default()
        };
        f.server()
            .emit_user_event(
                &first.account().user().to_string(),
                KnownEvent::Message(Box::new(message.clone())),
            )
            .await?;
        loop {
            let connection = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
            let count: i64 =
                connection.query_row("SELECT count(*) FROM notification_ledger", [], |row| {
                    row.get(0)
                })?;
            if count > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        first.auth().logout().await?;
        assert!(first.notifications().next_delivery().await.is_err());
        let auth = f.client().handle().auth(first.account().server())?;
        let replacement = login(&auth, "notify-a").await?;
        fixture::online(&replacement).await?;
        assert_ne!(first.generation(), replacement.generation());
        f.server()
            .emit_user_event(
                &replacement.account().user().to_string(),
                KnownEvent::Message(Box::new(message)),
            )
            .await?;
        assert!(
            tokio::time::timeout(
                Duration::from_millis(100),
                replacement.notifications().next_delivery()
            )
            .await
            .is_err()
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn final_background_handoff_revalidates_early_account_while_another_is_held() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let client = f.client().handle();
        client.background_sync(deadline()).await?;
        let at = timestamp();
        f.server()
            .set_notifications(
                first.account().user(),
                vec![row(
                    1,
                    channel,
                    second.account().user(),
                    first.account().user(),
                    &at,
                )],
            )
            .await?;
        let mut hold = f
            .server()
            .hold_next_rest(
                "GET",
                &format!("/users/{}/notifications", second.account().user()),
            )
            .await?;
        let copy = client.clone();
        let pending = tokio::spawn(async move { copy.background_sync(deadline()).await });
        hold.wait_started().await?;
        loop {
            let connection = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
            let count: i64 =
                connection.query_row("SELECT count(*) FROM notification_ledger", [], |row| {
                    row.get(0)
                })?;
            if count > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        client.remove_account(first.account()).await?;
        hold.release();
        assert!(pending.await??.deliveries.is_empty());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
