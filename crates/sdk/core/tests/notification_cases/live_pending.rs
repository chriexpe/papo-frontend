//! Loopback regressions for live Pending continuation, Absent vs corruption separation, and
//! multi-wake background continuation across restart and target switches.

use super::*;
use papo_api::wire::Message;
use papo_sdk::NotificationStatus;
use serde_json::json;

async fn seed_buried_row(
    f: &Fixture,
    session: &Session,
    author: Uuid,
    channel: Uuid,
    count: u128,
) -> Result<(String, Notification)> {
    let at = timestamp();
    let notif = row(1, channel, author, session.account().user(), &at);
    f.server()
        .set_notifications(session.account().user(), vec![notif.clone()])
        .await?;
    let messages = (1..=count)
        .map(|index| Message {
            id: Some(Uuid::from_u128(20_000 + index).to_string()),
            channel_id: Some(channel.to_string()),
            created_at: Some(at.clone()),
            ..Default::default()
        })
        .collect();
    f.server()
        .seed_messages(&channel.to_string(), messages)
        .await?;
    Ok((at, notif))
}

#[test]
fn live_pending_continues_without_fault_and_completes_once_evidence_resolves() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let (_, notif) = seed_buried_row(&f, &first, second.account().user(), channel, 501).await?;
        let message_id = notif.message_id.clone().ok_or("message")?;
        let before = f.server().read_request_counts().await.history_pages;
        // Emit only NewNotification (no Message event in `recent`) so live must paginate >4 pages.
        f.server()
            .emit_user_event(
                &first.account().user().to_string(),
                KnownEvent::NewNotification(Box::new(NewNotificationEvent {
                    id: notif.id.clone().ok_or("id")?,
                    user_id: second.account().user().to_string(),
                    message_id: message_id.clone(),
                    message_content: notif.message_content.clone().unwrap_or_default(),
                })),
            )
            .await?;
        let delivery = tokio::time::timeout(
            Duration::from_secs(10),
            first.notifications().next_delivery(),
        )
        .await??;
        assert_eq!(delivery.target.channel, channel);
        assert_eq!(delivery.target.message.to_string(), message_id);
        assert_eq!(
            *first.notifications().subscribe_status().borrow(),
            NotificationStatus::Ready
        );
        assert!(f.server().read_request_counts().await.history_pages - before >= 6);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn live_absent_stays_distinct_from_corrupt_history_page() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let at = timestamp();
        let notif = row(
            1,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        let message_id = notif.message_id.clone().ok_or("message")?;
        f.server()
            .set_notifications(first.account().user(), vec![notif.clone()])
            .await?;
        // Delete the message from channel history so the bounded scan ends Absent cleanly.
        assert!(f.server().delete_message(&message_id).await?);
        let before = f.server().read_request_counts().await.history_pages;
        f.server()
            .emit_user_event(
                &first.account().user().to_string(),
                KnownEvent::NewNotification(Box::new(NewNotificationEvent {
                    id: notif.id.clone().ok_or("id")?,
                    user_id: second.account().user().to_string(),
                    message_id: message_id.clone(),
                    message_content: notif.message_content.clone().unwrap_or_default(),
                })),
            )
            .await?;
        loop {
            if f.server().read_request_counts().await.history_pages > before {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(
            tokio::time::timeout(
                Duration::from_millis(150),
                first.notifications().next_delivery(),
            )
            .await
            .is_err()
        );
        assert_eq!(
            *first.notifications().subscribe_status().borrow(),
            NotificationStatus::Ready
        );

        // Now inject a genuinely corrupt history page (wrong channel_id): must fault.
        let notif2 = row(
            2,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        let message_id2 = notif2.message_id.clone().ok_or("message")?;
        f.server()
            .set_notifications(first.account().user(), vec![notif2.clone()])
            .await?;
        f.server()
            .queue_operation_response(
                "GET",
                &format!("/channels/{channel}/messages"),
                200,
                json!({
                    "channel_id": Uuid::from_u128(999_999).to_string(),
                    "messages": [],
                    "has_more": false
                }),
            )
            .await?;
        let mut status = first.notifications().subscribe_status();
        f.server()
            .emit_user_event(
                &first.account().user().to_string(),
                KnownEvent::NewNotification(Box::new(NewNotificationEvent {
                    id: notif2.id.clone().ok_or("id")?,
                    user_id: second.account().user().to_string(),
                    message_id: message_id2,
                    message_content: notif2.message_content.clone().unwrap_or_default(),
                })),
            )
            .await?;
        tokio::time::timeout(Duration::from_secs(5), async {
            while *status.borrow() != NotificationStatus::Fault {
                status.changed().await.unwrap();
            }
        })
        .await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn repeated_wakes_continue_beyond_four_pages_and_restart_repeats_the_scan() -> Result {
    let mut f = Fixture::new(papo_test_server::Config::default())?;
    let (account, other_account, channel) = f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let client = f.client().handle();
        client.push_wake(first.account(), deadline()).await?;
        seed_buried_row(&f, &first, second.account().user(), channel, 901).await?;
        let before = f.server().read_request_counts().await.history_pages;
        for _ in 0..2 {
            let report = client.push_wake(first.account(), deadline()).await?;
            assert_eq!(report.accounts[0].outcome, BackgroundOutcome::More);
            assert!(report.deliveries.is_empty());
        }
        assert_eq!(
            f.server().read_request_counts().await.history_pages - before,
            8
        );
        Ok::<_, Box<dyn std::error::Error>>((first.account(), second.account(), channel))
    })??;
    f.reopen()?;
    f.run(async {
        let client = f.client().handle();
        let session = client.session(account)?;
        let other = client.session(other_account)?;
        fixture::online(&session).await?;
        fixture::online(&other).await?;
        let before = f.server().read_request_counts().await.history_pages;
        for _ in 0..2 {
            let report = client.push_wake(account, deadline()).await?;
            assert_eq!(report.accounts[0].outcome, BackgroundOutcome::More);
            assert!(report.deliveries.is_empty());
        }
        let report = client.push_wake(account, deadline()).await?;
        assert_eq!(report.accounts[0].outcome, BackgroundOutcome::Complete);
        assert_eq!(report.deliveries.len(), 1);
        assert_eq!(report.deliveries[0].target.channel, channel);
        assert_eq!(
            f.server().read_request_counts().await.history_pages - before,
            10
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn changing_the_unresolved_target_restarts_its_bounded_scan() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let client = f.client().handle();
        client.push_wake(first.account(), deadline()).await?;
        let (at, original) =
            seed_buried_row(&f, &first, second.account().user(), channel, 501).await?;
        let before = f.server().read_request_counts().await.history_pages;
        assert_eq!(
            client
                .push_wake(first.account(), deadline())
                .await?
                .accounts[0]
                .outcome,
            BackgroundOutcome::More
        );
        let mut changed = row(
            1,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        changed.message_id = Some(Uuid::from_u128(9_999).to_string());
        f.server()
            .set_notifications(first.account().user(), vec![changed])
            .await?;
        assert_eq!(
            client
                .push_wake(first.account(), deadline())
                .await?
                .accounts[0]
                .outcome,
            BackgroundOutcome::More
        );
        f.server()
            .set_notifications(first.account().user(), vec![original])
            .await?;
        assert_eq!(
            client
                .push_wake(first.account(), deadline())
                .await?
                .accounts[0]
                .outcome,
            BackgroundOutcome::More
        );
        assert_eq!(
            f.server().read_request_counts().await.history_pages - before,
            12
        );
        let completed = client.push_wake(first.account(), deadline()).await?;
        assert_eq!(completed.accounts[0].outcome, BackgroundOutcome::Complete);
        assert_eq!(completed.deliveries.len(), 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn background_absent_stays_ready_while_corrupt_history_page_fails_without_cursor_burn() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let client = f.client().handle();
        client.push_wake(first.account(), deadline()).await?;
        let at = timestamp();
        let deleted = row(
            1,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        let deleted_id = deleted.message_id.clone().ok_or("message")?;
        let valid = row(
            2,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        f.server()
            .set_notifications(first.account().user(), vec![deleted, valid.clone()])
            .await?;
        assert!(f.server().delete_message(&deleted_id).await?);

        // Absent deleted row skips without faulting status; valid row still delivers.
        let report = client.push_wake(first.account(), deadline()).await?;
        assert_eq!(report.accounts[0].outcome, BackgroundOutcome::Complete);
        assert_eq!(report.deliveries.len(), 1);
        assert_eq!(
            Some(report.deliveries[0].target.message.to_string()),
            valid.message_id
        );
        assert_eq!(
            *first.notifications().subscribe_status().borrow(),
            NotificationStatus::Ready
        );

        // Corrupt history page fails without burning the inbox cursor; retry delivers cleanly.
        let third = row(
            3,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        f.server()
            .set_notifications(first.account().user(), vec![valid, third.clone()])
            .await?;
        f.server()
            .queue_operation_response(
                "GET",
                &format!("/channels/{channel}/messages"),
                200,
                json!({
                    "channel_id": Uuid::from_u128(777_777).to_string(),
                    "messages": [],
                    "has_more": false
                }),
            )
            .await?;
        let failed = client.push_wake(first.account(), deadline()).await?;
        assert_eq!(failed.accounts[0].outcome, BackgroundOutcome::Failed);
        assert!(failed.deliveries.is_empty());

        let recovered = client.push_wake(first.account(), deadline()).await?;
        assert_eq!(recovered.accounts[0].outcome, BackgroundOutcome::Complete);
        assert_eq!(recovered.deliveries.len(), 1);
        assert_eq!(
            Some(recovered.deliveries[0].target.message.to_string()),
            third.message_id
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn concurrent_buried_live_notifications_both_complete_without_ping_pong() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let at = timestamp();
        let row_a = row(
            1,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        let row_b = row(
            2,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        f.server()
            .set_notifications(first.account().user(), vec![row_a.clone(), row_b.clone()])
            .await?;
        let messages = (1..=501_u128)
            .map(|index| Message {
                id: Some(Uuid::from_u128(20_000 + index).to_string()),
                channel_id: Some(channel.to_string()),
                created_at: Some(at.clone()),
                ..Default::default()
            })
            .collect();
        f.server()
            .seed_messages(&channel.to_string(), messages)
            .await?;
        for notif in [&row_a, &row_b] {
            f.server()
                .emit_user_event(
                    &first.account().user().to_string(),
                    KnownEvent::NewNotification(Box::new(NewNotificationEvent {
                        id: notif.id.clone().ok_or("id")?,
                        user_id: second.account().user().to_string(),
                        message_id: notif.message_id.clone().ok_or("message")?,
                        message_content: notif.message_content.clone().unwrap_or_default(),
                    })),
                )
                .await?;
        }
        let mut delivered = std::collections::BTreeSet::new();
        for _ in 0..2 {
            let item = tokio::time::timeout(
                Duration::from_secs(10),
                first.notifications().next_delivery(),
            )
            .await??;
            delivered.insert(item.target.message.to_string());
        }
        assert_eq!(
            delivered,
            [row_a.message_id.ok_or("a")?, row_b.message_id.ok_or("b")?,].into()
        );
        assert_eq!(
            *first.notifications().subscribe_status().borrow(),
            NotificationStatus::Ready
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
