//! Partial background failures retain claims; live reply bursts reuse bounded backend evidence.
use super::*;

#[test]
fn later_history_failure_preserves_committed_deliveries_and_retry_progress() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let other: Uuid = f
            .server()
            .create_channel("other")
            .await?
            .id
            .ok_or("channel")?
            .parse()?;
        let client = f.client().handle();
        client.push_wake(first.account(), deadline()).await?;
        let at = timestamp();
        let first_row = row(
            1,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        let second_row = row(
            2,
            other,
            second.account().user(),
            first.account().user(),
            &at,
        );
        f.server()
            .set_notifications(
                first.account().user(),
                vec![first_row.clone(), second_row.clone()],
            )
            .await?;
        f.server()
            .fail_next_rest("GET", &format!("/channels/{other}/messages"), 503)
            .await?;
        let report = client.push_wake(first.account(), deadline()).await?;
        assert_eq!(report.accounts[0].outcome, BackgroundOutcome::Failed);
        assert_eq!(report.deliveries.len(), 1);
        assert_eq!(
            Some(report.deliveries[0].target.message.to_string()),
            first_row.message_id
        );
        let connection = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
        assert_eq!(
            connection.query_row("SELECT count(*) FROM notification_ledger", [], |r| r
                .get::<_, i64>(0))?,
            1
        );
        let retry = client.push_wake(first.account(), deadline()).await?;
        assert_eq!(retry.accounts[0].outcome, BackgroundOutcome::Complete);
        assert_eq!(retry.deliveries.len(), 1);
        assert_eq!(
            Some(retry.deliveries[0].target.message.to_string()),
            second_row.message_id
        );
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

async fn reply_event(
    f: &Fixture,
    first: &Session,
    second: &Session,
    record: &Notification,
) -> Result {
    f.server()
        .emit_user_event(
            &first.account().user().to_string(),
            KnownEvent::Message(Box::new(MessageEvent {
                id: record.message_id.clone().ok_or("message")?,
                channel_id: record.channel_id.clone().ok_or("channel")?,
                author_id: second.account().user().to_string(),
                content: String::new(),
                created_at: record.created_at.clone().ok_or("time")?,
                ..Default::default()
            })),
        )
        .await?;
    f.server()
        .emit_user_event(
            &first.account().user().to_string(),
            KnownEvent::NewNotification(Box::new(NewNotificationEvent {
                id: record.id.clone().ok_or("notification")?,
                user_id: second.account().user().to_string(),
                message_id: record.message_id.clone().ok_or("message")?,
                message_content: String::new(),
            })),
        )
        .await?;
    Ok(())
}

#[test]
fn live_reply_backlog_shares_one_inbox_page_and_uncached_event_fetches_evidence() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let at = timestamp();
        let records: Vec<_> = (1..=16)
            .map(|i| {
                let mut record = row(
                    i,
                    channel,
                    second.account().user(),
                    first.account().user(),
                    &at,
                );
                record.message_content = Some(String::new());
                record
            })
            .collect();
        f.server()
            .set_notifications(first.account().user(), records.clone())
            .await?;
        let path = format!("/users/{}/notifications", first.account().user());
        let before = f.server().read_request_counts().await;
        let mut hold = f.server().hold_next_rest("GET", &path).await?;
        reply_event(&f, &first, &second, &records[0]).await?;
        hold.wait_started().await?;
        for record in &records[1..] {
            reply_event(&f, &first, &second, record).await?;
        }
        // This later resource publication proves the gateway has queued the entire burst.
        let barrier = f
            .server()
            .create_channel("barrier")
            .await?
            .id
            .ok_or("channel")?;
        let mut resources = first.subscribe_resources();
        while !resources
            .borrow()
            .channels
            .iter()
            .any(|c| c.id.as_ref() == Some(&barrier))
        {
            resources.changed().await?;
        }
        hold.release();
        let mut messages = std::collections::BTreeSet::new();
        for _ in 0..records.len() {
            messages.insert(first.notifications().next_delivery().await?.target.message);
        }
        assert_eq!(messages.len(), records.len());
        assert_eq!(
            f.server().read_request_counts().await.notification_pages - before.notification_pages,
            1
        );
        let mut next = row(
            17,
            channel,
            second.account().user(),
            first.account().user(),
            &at,
        );
        next.message_content = Some(String::new());
        f.server()
            .set_notifications(first.account().user(), vec![next.clone()])
            .await?;
        reply_event(&f, &first, &second, &next).await?;
        first.notifications().next_delivery().await?;
        assert_eq!(
            f.server().read_request_counts().await.notification_pages - before.notification_pages,
            2
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn cancellation_after_an_earlier_claim_still_suppresses_platform_handoff() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let other: Uuid = f
            .server()
            .create_channel("other")
            .await?
            .id
            .ok_or("channel")?
            .parse()?;
        let client = f.client().handle();
        client.background_sync(deadline()).await?;
        let at = timestamp();
        f.server()
            .set_notifications(
                first.account().user(),
                vec![
                    row(
                        1,
                        channel,
                        second.account().user(),
                        first.account().user(),
                        &at,
                    ),
                    row(
                        2,
                        other,
                        second.account().user(),
                        first.account().user(),
                        &at,
                    ),
                ],
            )
            .await?;
        let mut hold = f
            .server()
            .hold_next_rest("GET", &format!("/channels/{other}/messages"))
            .await?;
        let cancel = papo_sdk::BackgroundCancellation::new();
        let token = cancel.clone();
        let copy = client.clone();
        let pending =
            tokio::spawn(async move { copy.background_sync_cancelable(deadline(), token).await });
        hold.wait_started().await?;
        let connection = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
        assert_eq!(
            connection.query_row("SELECT count(*) FROM notification_ledger", [], |r| r
                .get::<_, i64>(0))?,
            1
        );
        cancel.cancel();
        let report = pending.await??;
        assert!(report.deliveries.is_empty());
        assert_eq!(
            report
                .accounts
                .iter()
                .find(|a| a.account == first.account())
                .ok_or("account")?
                .outcome,
            BackgroundOutcome::Interrupted
        );
        hold.release();
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn completed_local_read_invalidates_inflight_and_backlog_reply_evidence() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, channel) = setup(&f).await?;
        let at = timestamp();
        let records: Vec<_> = (1..=3)
            .map(|index| {
                let mut record = row(
                    index,
                    channel,
                    second.account().user(),
                    first.account().user(),
                    &at,
                );
                record.message_content = Some(String::new());
                record
            })
            .collect();
        f.server()
            .set_notifications(first.account().user(), records.clone())
            .await?;
        let path = format!("/users/{}/notifications", first.account().user());
        let mut hold = f.server().hold_next_rest("GET", &path).await?;
        reply_event(&f, &first, &second, &records[0]).await?;
        hold.wait_started().await?;
        let inbox = first.notifications().inbox(None).await?;
        let read = inbox
            .records
            .iter()
            .find(|r| r.record.message_id == records[1].message_id)
            .ok_or("record")?
            .clone();
        first.notifications().mark_read(&[read]).await?;
        reply_event(&f, &first, &second, &records[1]).await?;
        reply_event(&f, &first, &second, &records[2]).await?;
        hold.release();
        let mut delivered = std::collections::BTreeSet::new();
        for _ in 0..2 {
            delivered.insert(
                first
                    .notifications()
                    .next_delivery()
                    .await?
                    .target
                    .message
                    .to_string(),
            );
        }
        assert_eq!(
            delivered,
            [
                records[0].message_id.clone().ok_or("message")?,
                records[2].message_id.clone().ok_or("message")?
            ]
            .into()
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
