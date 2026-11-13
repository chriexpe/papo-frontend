//! Public notification races, quiet baselines and account lifecycle over loopback; never claims real FCM delivery.
#![cfg(feature = "test-support")]
mod fixture;
use fixture::{Fixture, Result, login, register};
use papo_api::{
    gateway::{KnownEvent, MessageEvent, NewNotificationEvent},
    wire::Notification,
};
use papo_sdk::{BackgroundOutcome, PushDevice, PushRegistration, Session, UnixMicros, Uuid};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn timestamp() -> String {
    let micros = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros(),
    )
    .unwrap();
    chrono::DateTime::from_timestamp_micros(micros)
        .unwrap()
        .to_rfc3339()
}
async fn setup(f: &Fixture) -> Result<(Session, Session, Uuid)> {
    let client = f.client().handle();
    let server = client
        .add_server(&f.server().base_url(), UnixMicros(0))
        .await?;
    let auth = client.auth(server)?;
    register(&auth, "notify-a").await?;
    let first = login(&auth, "notify-a").await?;
    fixture::online(&first).await?;
    let auth = client.auth(server)?;
    register(&auth, "notify-b").await?;
    let second = login(&auth, "notify-b").await?;
    fixture::online(&second).await?;
    let channel = f.server().create_channel("fixture").await?;
    let channel: Uuid = channel.id.ok_or("missing channel")?.parse()?;
    for session in [&first, &second] {
        let mut resources = session.subscribe_resources();
        loop {
            if resources
                .borrow()
                .channels
                .iter()
                .any(|c| c.id.as_deref().and_then(|id| id.parse().ok()) == Some(channel))
            {
                break;
            }
            resources.changed().await?;
        }
    }
    Ok((first, second, channel))
}
fn row(index: u128, channel: Uuid, author: Uuid, user: Uuid, at: &str) -> Notification {
    Notification {
        id: Some(Uuid::from_u128(index + 1000).to_string()),
        message_id: Some(Uuid::from_u128(index + 10000).to_string()),
        channel_id: Some(channel.to_string()),
        author_id: Some(author.to_string()),
        message_content: Some(format!("@mention(<@{user}>)")),
        read: Some(false),
        created_at: Some(at.into()),
        ..Default::default()
    }
}
fn deadline() -> tokio::time::Instant {
    tokio::time::Instant::now() + Duration::from_secs(10)
}
#[test]
fn initial_inbox_is_quiet_then_exact_tie_pages_catch_up_once() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let at = timestamp();
        let client = f.client().handle();
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
        assert!(
            client
                .background_sync(deadline())
                .await?
                .deliveries
                .is_empty()
        );
        let rows = (1..=111)
            .map(|i| {
                row(
                    i,
                    channel,
                    second.account().user(),
                    first.account().user(),
                    &at,
                )
            })
            .collect();
        f.server()
            .set_notifications(first.account().user(), rows)
            .await?;
        let report = client.background_sync(deadline()).await?;
        assert_eq!(report.deliveries.len(), 110);
        assert!(
            report
                .accounts
                .iter()
                .all(|a| a.outcome == BackgroundOutcome::Complete)
        );
        assert!(
            client
                .background_sync(deadline())
                .await?
                .deliveries
                .is_empty()
        );
        let page = first.notifications().inbox(None).await?;
        assert_eq!(page.records.len(), 100);
        assert!(page.records.iter().all(|r| r.actual_mention));
        let older = first.notifications().inbox(page.next).await?;
        assert_eq!(older.records.len(), 11);
        assert!(older.next.is_none());
        first.notifications().mark_read(&older.records).await?;
        let target = first.notifications().target(&older.records[0])?;
        assert_eq!(target.account, first.account());
        assert_eq!(target.channel, channel);
        assert!(second.notifications().target(&older.records[0]).is_err());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
#[test]
fn push_gateway_and_background_share_one_canonical_claim() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let client = f.client().handle();
        assert!(
            client
                .background_sync(deadline())
                .await?
                .deliveries
                .is_empty()
        );
        let at = timestamp();
        let notification = row(
            1,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        f.server()
            .set_notifications(first.account().user(), vec![notification.clone()])
            .await?;
        let message = MessageEvent {
            id: notification.message_id.clone().ok_or("id")?,
            channel_id: channel.to_string(),
            author_id: second.account().user().to_string(),
            content: notification.message_content.clone().ok_or("content")?,
            created_at: at,
            ..Default::default()
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
                KnownEvent::NewNotification(Box::new(NewNotificationEvent {
                    id: Uuid::from_u128(9000).to_string(),
                    user_id: second.account().user().to_string(),
                    message_id: notification.message_id.clone().ok_or("id")?,
                    message_content: notification.message_content.clone().ok_or("content")?,
                })),
            )
            .await?;
        let (push, background) = tokio::join!(
            client.push_wake(first.account(), deadline()),
            client.background_sync(deadline())
        );
        let mut count = push?.deliveries.len() + background?.deliveries.len();
        if let Ok(delivery) = tokio::time::timeout(
            Duration::from_millis(250),
            first.notifications().next_delivery(),
        )
        .await
        {
            first.notifications().validate_target(&delivery?.target)?;
            count += 1;
        }
        assert_eq!(count, 1);
        assert!(
            tokio::time::timeout(
                Duration::from_millis(100),
                first.notifications().next_delivery()
            )
            .await
            .is_err()
        );
        let connection = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
        assert_eq!(
            connection.query_row("SELECT count(*) FROM notification_ledger", [], |r| r
                .get::<_, i64>(0))?,
            1
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
#[test]
fn preemption_cancellation_deadline_and_logout_fence_background() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, _, _) = setup(&f).await?;
        let client = f.client().handle();
        let interactive = client.interactive_sync();
        let report = client.background_sync(deadline()).await?;
        assert!(
            report
                .accounts
                .iter()
                .all(|a| a.outcome == BackgroundOutcome::Preempted)
        );
        drop(interactive);
        let cancel = papo_sdk::BackgroundCancellation::new();
        cancel.cancel();
        let report = client
            .background_sync_cancelable(deadline(), cancel)
            .await?;
        assert!(
            report
                .accounts
                .iter()
                .all(|a| a.outcome == BackgroundOutcome::Interrupted)
        );
        let mut hold = f.server().hold_next_rest("GET", "/channels").await?;
        let work_client = client.clone();
        let pending =
            tokio::spawn(async move { work_client.push_wake(first.account(), deadline()).await });
        hold.wait_started().await?;
        let guard = client.interactive_sync();
        let report = pending.await??;
        assert!(report.deliveries.is_empty());
        assert_eq!(report.accounts[0].outcome, BackgroundOutcome::Preempted);
        hold.release();
        drop(guard);
        let report = client.background_sync(tokio::time::Instant::now()).await?;
        assert!(report.deliveries.is_empty());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
fn device(token: &str) -> PushDevice {
    PushDevice {
        token: token.into(),
        platform: "android".into(),
        device_name: None,
        provider: "fcm".into(),
    }
}
#[test]
fn push_ownership_transfers_and_old_account_cannot_unregister_new_owner() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, _) = setup(&f).await?;
        assert_eq!(
            first
                .notifications()
                .register_push(device("synthetic-token"))
                .await?,
            PushRegistration::Registered
        );
        assert_eq!(
            second
                .notifications()
                .register_push(device("synthetic-token"))
                .await?,
            PushRegistration::Registered
        );
        assert_eq!(
            first.notifications().unregister_push().await?,
            PushRegistration::Removed
        );
        assert_eq!(
            f.server().push_owner("synthetic-token").await,
            Some(second.account().user().to_string())
        );
        assert_eq!(
            second
                .notifications()
                .unregister_push_device(device("synthetic-token"))
                .await?,
            PushRegistration::Removed
        );
        assert!(f.server().push_owner("synthetic-token").await.is_none());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn policy_survives_restart_and_host_token_can_be_removed_without_local_ownership() -> Result {
    let mut f = Fixture::new(papo_test_server::Config::default())?;
    let (account, channel) = f.run(async {
        let (first, _, channel) = setup(&f).await?;
        let mut settings = first.notifications().settings().await?;
        settings.muted.insert(channel);
        first.notifications().set_settings(settings).await?;
        first
            .notifications()
            .register_push(device("synthetic-token"))
            .await?;
        Ok::<_, Box<dyn std::error::Error>>((first.account(), channel))
    })??;
    f.reopen()?;
    f.run(async {
        let session = f.client().handle().session(account)?;
        fixture::online(&session).await?;
        assert!(
            session
                .notifications()
                .settings()
                .await?
                .muted
                .contains(&channel)
        );
        assert_eq!(
            session.notifications().unregister_push().await?,
            PushRegistration::Unknown
        );
        assert_eq!(
            session
                .notifications()
                .unregister_push_device(device("synthetic-token"))
                .await?,
            PushRegistration::Removed
        );
        assert!(f.store.calls(papo_sdk::secret::test_store::Operation::Read) > 0);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[path = "notification_cases/policy.rs"]
mod policy_cases;

#[path = "notification_cases/push_races.rs"]
mod push_races;

#[path = "notification_cases/continuation.rs"]
mod continuation;
