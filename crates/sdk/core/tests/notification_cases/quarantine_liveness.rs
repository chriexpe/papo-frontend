//! Loopback regressions for quarantine liveness: non-blocking background catch-up with frozen
//! durable cursor and bounded live deferral eviction without faulting.

use super::{
    access_and_admission::{quarantine_channel, release_quarantine},
    read_evidence::{DurableKind, ledger_rows, seed_read_points, timestamp_offset},
    *,
};
use papo_api::wire::Message;
use papo_sdk::{ChannelNotificationMode, InboxCursor, MessageKey, NotificationStatus};
use rusqlite::OptionalExtension;

fn stored_cursor(f: &Fixture, session: &Session) -> Result<Option<InboxCursor>> {
    let connection = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
    let raw: Option<Option<Vec<u8>>> = connection
        .query_row(
            "SELECT cursor FROM notification_state WHERE server_id=?1 AND user_id=?2",
            rusqlite::params![
                session.account().server().as_bytes(),
                session.account().user().as_bytes()
            ],
            |row| row.get(0),
        )
        .optional()?;
    raw.flatten()
        .map(|bytes| serde_json::from_slice(&bytes).map_err(Into::into))
        .transpose()
}

async fn create_visible_channel(f: &Fixture, session: &Session, name: &str) -> Result<Uuid> {
    let channel: Uuid = f
        .server()
        .create_channel(name)
        .await?
        .id
        .ok_or("channel")?
        .parse()?;
    let mut resources = session.subscribe_resources();
    while !resources.borrow().channels.iter().any(|c| c.id == channel) {
        resources.changed().await?;
    }
    Ok(channel)
}

async fn seed_quarantined_and_healthy_inbox(
    f: &Fixture,
    first: &Session,
    second: &Session,
    quarantined: Uuid,
    healthy: Uuid,
    already_read: bool,
) -> Result<(InboxCursor, Notification, Notification, InboxCursor)> {
    let client = f.client().handle();
    let t0 = timestamp_offset(-6);
    let t1 = timestamp_offset(-4);
    let t2 = timestamp_offset(-2);
    let baseline_row = row(
        1,
        healthy,
        second.account().user(),
        first.account().user(),
        &t0,
    );
    let baseline_cursor = InboxCursor {
        created_at: t0,
        id: baseline_row.id.clone().ok_or("baseline id")?,
    };
    f.server()
        .set_notifications(first.account().user(), vec![baseline_row.clone()])
        .await?;
    let initial = client.push_wake(first.account(), deadline()).await?;
    assert_eq!(initial.accounts[0].outcome, BackgroundOutcome::Complete);
    assert!(initial.deliveries.is_empty());
    assert_eq!(stored_cursor(f, first)?, Some(baseline_cursor.clone()));

    quarantine_channel(f, first, second.account().user(), quarantined).await?;
    let deferred_row = row(
        2,
        quarantined,
        second.account().user(),
        first.account().user(),
        &t1,
    );
    let healthy_row = row(
        3,
        healthy,
        second.account().user(),
        first.account().user(),
        &t2,
    );
    let final_cursor = InboxCursor {
        created_at: t2,
        id: healthy_row.id.clone().ok_or("healthy id")?,
    };
    if already_read {
        let boundary =
            MessageKey::parse(&timestamp_offset(5), &Uuid::from_u128(90_000).to_string())?;
        seed_read_points(
            f,
            first,
            quarantined,
            DurableKind::Local,
            boundary,
            Uuid::from_u128(90_001),
        )?;
    }
    f.server()
        .set_notifications(
            first.account().user(),
            vec![baseline_row, deferred_row.clone(), healthy_row.clone()],
        )
        .await?;
    Ok((baseline_cursor, deferred_row, healthy_row, final_cursor))
}

#[test]
fn quarantined_row_does_not_block_later_healthy_row_and_freezes_durable_cursor() -> Result {
    for already_read in [false, true] {
        let f = Fixture::new(papo_test_server::Config::default())?;
        f.run(async {
            let (first, second, quarantined) = setup(&f).await?;
            let healthy = create_visible_channel(&f, &first, "healthy").await?;
            let client = f.client().handle();
            let (baseline_cursor, deferred_row, healthy_row, final_cursor) =
                seed_quarantined_and_healthy_inbox(
                    &f,
                    &first,
                    &second,
                    quarantined,
                    healthy,
                    already_read,
                )
                .await?;

            // First wake while quarantined: healthy row delivers, but durable cursor stays frozen
            // before the deferred row so it will be revisited once quarantine clears.
            let report = client.background_sync(deadline()).await?;
            assert_eq!(
                report
                    .accounts
                    .iter()
                    .find(|a| a.account == first.account())
                    .ok_or("first")?
                    .outcome,
                BackgroundOutcome::More
            );
            assert_eq!(report.deliveries.len(), 1);
            assert_eq!(report.deliveries[0].target.channel, healthy);
            assert_eq!(
                Some(report.deliveries[0].target.message.to_string()),
                healthy_row.message_id
            );
            assert_eq!(ledger_rows(&f)?, 1);
            assert_eq!(stored_cursor(&f, &first)?, Some(baseline_cursor.clone()));

            // Second wake while still quarantined: healthy row is deduplicated by the ledger,
            // deferred row stays unclaimed and durable cursor stays frozen.
            let retry = client.push_wake(first.account(), deadline()).await?;
            assert_eq!(retry.accounts[0].outcome, BackgroundOutcome::More);
            assert!(retry.deliveries.is_empty());
            assert_eq!(ledger_rows(&f)?, 1);
            assert_eq!(stored_cursor(&f, &first)?, Some(baseline_cursor));

            // Clearing quarantine delivers (or suppresses if read) the deferred row, claims it,
            // and advances the durable cursor past the already-claimed healthy row.
            release_quarantine(&f, &first, quarantined).await?;
            let after = client.push_wake(first.account(), deadline()).await?;
            assert_eq!(after.accounts[0].outcome, BackgroundOutcome::Complete);
            if already_read {
                assert!(after.deliveries.is_empty());
            } else {
                assert_eq!(after.deliveries.len(), 1);
                assert_eq!(after.deliveries[0].target.channel, quarantined);
                assert_eq!(
                    Some(after.deliveries[0].target.message.to_string()),
                    deferred_row.message_id
                );
            }
            assert_eq!(ledger_rows(&f)?, 2);
            assert_eq!(stored_cursor(&f, &first)?, Some(final_cursor));

            let quiet = client.push_wake(first.account(), deadline()).await?;
            assert_eq!(quiet.accounts[0].outcome, BackgroundOutcome::Complete);
            assert!(quiet.deliveries.is_empty());
            Ok::<_, Box<dyn std::error::Error>>(())
        })??;
        f.finish()?;
    }
    Ok(())
}

async fn emit_dm_batch_and_barrier(
    f: &Fixture,
    recipient: &Session,
    author: Uuid,
    dm_channel: Uuid,
    healthy: Uuid,
    range: std::ops::Range<u128>,
    barrier_id: Uuid,
) -> Result {
    let at = timestamp();
    for index in range {
        // Ordinary DM messages (no mention) are eligible and defer while the DM is quarantined.
        f.server()
            .emit_user_event(
                &recipient.account().user().to_string(),
                KnownEvent::Message(Box::new(MessageEvent {
                    id: Uuid::from_u128(50_000 + index).to_string(),
                    channel_id: dm_channel.to_string(),
                    author_id: author.to_string(),
                    content: format!("dm-{index}"),
                    created_at: at.clone(),
                    ..Default::default()
                })),
            )
            .await?;
    }
    // A FIFO barrier mention on a healthy channel guarantees the live task has drained all prior
    // DM events from the gateway mpsc channel before assertions inspect queue diagnostics.
    f.server()
        .emit_user_event(
            &recipient.account().user().to_string(),
            KnownEvent::Message(Box::new(MessageEvent {
                id: barrier_id.to_string(),
                channel_id: healthy.to_string(),
                author_id: author.to_string(),
                content: format!("@mention(<@{}>)", recipient.account().user()),
                created_at: at,
                ..Default::default()
            })),
        )
        .await?;
    let delivered = tokio::time::timeout(
        Duration::from_secs(5),
        recipient.notifications().next_delivery(),
    )
    .await??;
    assert_eq!(delivered.target.message, barrier_id);
    Ok(())
}

async fn release_and_drain_retained(
    f: &Fixture,
    first: &Session,
    dm_channel: Uuid,
    count: usize,
) -> Result<std::collections::BTreeSet<Uuid>> {
    let mut settings = first.notifications().settings().await?;
    settings
        .channels
        .insert(dm_channel, ChannelNotificationMode::All);
    first.notifications().set_settings(settings).await?;

    let notifications = first.notifications();
    let drain = tokio::spawn(async move {
        let mut delivered = std::collections::BTreeSet::new();
        for _ in 0..count {
            let item = tokio::time::timeout(Duration::from_secs(5), notifications.next_delivery())
                .await??;
            delivered.insert(item.target.message);
        }
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(delivered)
    });
    release_quarantine(f, first, dm_channel).await?;
    Ok(drain
        .await
        .map_err(|err| err.to_string())?
        .map_err(|err| err.to_string())?)
}

#[test]
fn quarantined_dm_receiving_over_sixty_four_messages_drops_oldest_and_stays_ready() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, dm_channel) = setup(&f).await?;
        let healthy = create_visible_channel(&f, &first, "healthy").await?;
        quarantine_channel(&f, &first, second.account().user(), dm_channel).await?;

        // Fill the 64-entry deferred queue in two 32-message batches without overflowing `rx`.
        for (range, barrier) in [(0..32, 60_001u128), (32..64, 60_002u128)] {
            emit_dm_batch_and_barrier(
                &f,
                &first,
                second.account().user(),
                dm_channel,
                healthy,
                range,
                Uuid::from_u128(barrier),
            )
            .await?;
        }
        assert_eq!(
            *first.notifications().subscribe_status().borrow(),
            NotificationStatus::Ready
        );
        assert_eq!(first.notifications().diagnostics().deferred_evictions, 0);

        // Push 6 more messages past the 64-entry limit: oldest unclaimed entries drop with a
        // diagnostic counter and status stays Ready.
        emit_dm_batch_and_barrier(
            &f,
            &first,
            second.account().user(),
            dm_channel,
            healthy,
            64..70,
            Uuid::from_u128(60_003),
        )
        .await?;
        assert_eq!(
            *first.notifications().subscribe_status().borrow(),
            NotificationStatus::Ready
        );
        let diagnostics = first.notifications().diagnostics();
        assert_eq!(diagnostics.deferred_evictions, 6);
        assert_eq!(diagnostics.lookup_evictions, 0);
        // Only the 3 healthy-channel barriers were claimed; none of the 70 quarantined DM rows were.
        assert_eq!(ledger_rows(&f)?, 3);

        // Keep `dm_channel` eligible after releasing quarantine, then verify all 64 retained
        // messages (`50_006..50_070`) deliver and none of the 6 evicted entries (`50_000..50_006`) do.
        let delivered = release_and_drain_retained(&f, &first, dm_channel, 64).await?;
        let expected: std::collections::BTreeSet<Uuid> =
            (6..70u128).map(|i| Uuid::from_u128(50_000 + i)).collect();
        assert_eq!(delivered, expected);
        assert_eq!(ledger_rows(&f)?, 67);

        let post_id = Uuid::from_u128(60_004);
        f.server()
            .emit_user_event(
                &first.account().user().to_string(),
                KnownEvent::Message(Box::new(MessageEvent {
                    id: post_id.to_string(),
                    channel_id: dm_channel.to_string(),
                    author_id: second.account().user().to_string(),
                    content: format!("@mention(<@{}>)", first.account().user()),
                    created_at: timestamp(),
                    ..Default::default()
                })),
            )
            .await?;
        let post = tokio::time::timeout(
            Duration::from_secs(5),
            first.notifications().next_delivery(),
        )
        .await??;
        assert_eq!(post.target.message, post_id);
        assert_eq!(ledger_rows(&f)?, 68);
        assert_eq!(
            *first.notifications().subscribe_status().borrow(),
            NotificationStatus::Ready
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn buried_healthy_row_behind_quarantined_row_completes_across_wakes_without_livelock() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (first, second, quarantined) = setup(&f).await?;
        let healthy_near = create_visible_channel(&f, &first, "healthy-near").await?;
        let healthy_buried = create_visible_channel(&f, &first, "healthy-buried").await?;
        let client = f.client().handle();
        client.push_wake(first.account(), deadline()).await?;
        quarantine_channel(&f, &first, second.account().user(), quarantined).await?;

        let (t1, t2, t3) = (
            timestamp_offset(-6),
            timestamp_offset(-4),
            timestamp_offset(-2),
        );
        let q_row = row(
            1,
            quarantined,
            second.account().user(),
            first.account().user(),
            &t1,
        );
        let near_row = row(
            2,
            healthy_near,
            second.account().user(),
            first.account().user(),
            &t2,
        );
        let buried_row = row(
            3,
            healthy_buried,
            second.account().user(),
            first.account().user(),
            &t3,
        );
        f.server()
            .set_notifications(
                first.account().user(),
                vec![q_row.clone(), near_row.clone(), buried_row.clone()],
            )
            .await?;
        let newer_at = timestamp_offset(0);
        let filler = (1..=450u128)
            .map(|index| Message {
                id: Some(Uuid::from_u128(70_000 + index).to_string()),
                channel_id: Some(healthy_buried.to_string()),
                created_at: Some(newer_at.clone()),
                ..Default::default()
            })
            .collect();
        f.server()
            .seed_messages(&healthy_buried.to_string(), filler)
            .await?;

        // Wake 1: delivers `healthy_near` and scans pages 1..4 of `healthy_buried` (`Pending`).
        let wake1 = client.push_wake(first.account(), deadline()).await?;
        assert_eq!(wake1.accounts[0].outcome, BackgroundOutcome::More);
        assert_eq!(wake1.deliveries.len(), 1);
        assert_eq!(
            Some(wake1.deliveries[0].target.message.to_string()),
            near_row.message_id
        );

        // Wake 2 (while `quarantined` is STILL quarantined): skips `q_row` and already-claimed
        // `near_row` without clobbering `CanonicalPoints`, finishing `healthy_buried` on page 5.
        let wake2 = client.push_wake(first.account(), deadline()).await?;
        assert_eq!(wake2.accounts[0].outcome, BackgroundOutcome::More);
        assert_eq!(wake2.deliveries.len(), 1);
        assert_eq!(
            Some(wake2.deliveries[0].target.message.to_string()),
            buried_row.message_id
        );

        // Clearing quarantine delivers `q_row` and completes the inbox.
        release_quarantine(&f, &first, quarantined).await?;
        let wake3 = client.push_wake(first.account(), deadline()).await?;
        assert_eq!(wake3.accounts[0].outcome, BackgroundOutcome::Complete);
        assert_eq!(wake3.deliveries.len(), 1);
        assert_eq!(
            Some(wake3.deliveries[0].target.message.to_string()),
            q_row.message_id
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn unrecognized_or_absent_mode_retries_the_same_inbox_identity_after_revalidation() -> Result {
    let fixture = Fixture::new(papo_test_server::Config::default())?;
    fixture.run(async {
        let (session, sender, channel) = setup(&fixture).await?;
        let client = fixture.client().handle();
        let account = session.account();
        let baseline = row(
            1,
            channel,
            sender.account().user(),
            account.user(),
            &timestamp_offset(-5),
        );
        fixture
            .server()
            .set_notifications(account.user(), vec![baseline.clone()])
            .await?;
        client.push_wake(account, deadline()).await?;
        let previous = stored_cursor(&fixture, &session)?.ok_or("baseline cursor")?;
        let pending = row(
            2,
            channel,
            sender.account().user(),
            account.user(),
            &timestamp_offset(-1),
        );
        fixture
            .server()
            .set_notifications(account.user(), vec![baseline, pending.clone()])
            .await?;
        for mode in [None, Some("future")] {
            let mut record = serde_json::json!({"id":channel,"type":"text","permissions":[]});
            if let Some(raw) = mode {
                record["notification_settings"] = serde_json::json!(raw);
            }
            fixture
                .server()
                .queue_operation_response(
                    "GET",
                    "/channels",
                    200,
                    serde_json::json!({"channels":[record]}),
                )
                .await?;
            let report = client.push_wake(account, deadline()).await?;
            assert_eq!(report.accounts[0].outcome, BackgroundOutcome::More);
            assert!(report.deliveries.is_empty());
            assert_eq!(ledger_rows(&fixture)?, 0);
            assert_eq!(stored_cursor(&fixture, &session)?.as_ref(), Some(&previous));
        }
        let retried = client.push_wake(account, deadline()).await?;
        assert_eq!(retried.accounts[0].outcome, BackgroundOutcome::Complete);
        assert_eq!(retried.deliveries.len(), 1);
        assert_eq!(ledger_rows(&fixture)?, 1);
        assert_eq!(
            stored_cursor(&fixture, &session)?.map(|cursor| cursor.id),
            pending.id
        );
        assert!(
            client
                .push_wake(account, deadline())
                .await?
                .deliveries
                .is_empty()
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
