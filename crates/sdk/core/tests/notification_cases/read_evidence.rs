//! Loopback regressions for reading-owned cold durable evidence (local, uploaded, known,
//! unresolved exact-ID and independent inbox read flag).

use super::*;
use papo_sdk::MessageKey;
use serde_json::json;

pub(super) fn ledger_rows(f: &Fixture) -> Result<i64> {
    let connection = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
    Ok(
        connection.query_row("SELECT count(*) FROM notification_ledger", [], |row| {
            row.get(0)
        })?,
    )
}

pub(super) fn timestamp_offset(delta_secs: i64) -> String {
    let micros = i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros(),
    )
    .unwrap()
        + delta_secs * 1_000_000;
    chrono::DateTime::from_timestamp_micros(micros)
        .unwrap()
        .to_rfc3339()
}

#[derive(Clone, Copy)]
pub(super) enum DurableKind {
    Local,
    Uploaded,
    Known,
    UnresolvedExactId,
}

pub(super) fn seed_read_points(
    f: &Fixture,
    session: &Session,
    channel: Uuid,
    kind: DurableKind,
    boundary: MessageKey,
    exact_id: Uuid,
) -> Result {
    let database = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
    let (local, uploaded): (Option<Vec<u8>>, Option<Vec<u8>>) = match kind {
        DurableKind::Local => (Some(serde_json::to_vec(&boundary)?), None),
        DurableKind::Uploaded => (
            None,
            Some(serde_json::to_vec(&json!({
                "message_id": boundary.message_id.to_string(),
                "recorded_at": null,
                "created_at_us": boundary.created_at_us,
                "known_point": null
            }))?),
        ),
        DurableKind::Known => (
            None,
            Some(serde_json::to_vec(&json!({
                "message_id": Uuid::from_u128(99_999).to_string(),
                "recorded_at": "2099-01-01T00:00:00Z",
                "created_at_us": null,
                "known_point": boundary
            }))?),
        ),
        DurableKind::UnresolvedExactId => (
            None,
            Some(serde_json::to_vec(&json!({
                "message_id": exact_id.to_string(),
                "recorded_at": "2099-01-01T00:00:00Z",
                "created_at_us": null,
                "known_point": null
            }))?),
        ),
    };
    database.execute(
        "INSERT INTO read_points(server_id,user_id,channel_id,local_point,uploaded_point)
         VALUES(?1,?2,?3,?4,?5)",
        rusqlite::params![
            session.account().server().as_bytes(),
            session.account().user().as_bytes(),
            channel.as_bytes(),
            local,
            uploaded
        ],
    )?;
    Ok(())
}

#[test]
fn cold_background_and_push_respect_durable_read_points_and_deliver_unread_counterparts() -> Result
{
    let kinds = [
        DurableKind::Local,
        DurableKind::Uploaded,
        DurableKind::Known,
        DurableKind::UnresolvedExactId,
    ];
    for push in [false, true] {
        for kind in kinds {
            for unread_counterpart in [false, true] {
                let f = Fixture::new(papo_test_server::Config::default())?;
                f.run(async {
                    let client = f.client().handle();
                    let server = client
                        .add_server(&f.server().base_url(), UnixMicros(0))
                        .await?;
                    let auth = client.auth(server)?;
                    register(&auth, "cold-durable").await?;
                    // Hold initial foreground GET /channels so reading reconcile never runs.
                    let mut hold = f.server().hold_next_rest("GET", "/channels").await?;
                    let session = login(&auth, "cold-durable").await?;
                    hold.wait_started().await?;
                    let channel: Uuid = f
                        .server()
                        .create_channel("fixture")
                        .await?
                        .id
                        .ok_or("channel")?
                        .parse()?;
                    // Establish quiet inbox baseline before seeding durable read evidence.
                    client.push_wake(session.account(), deadline()).await?;
                    let boundary_at = timestamp_offset(-2);
                    let candidate_at = if unread_counterpart {
                        timestamp_offset(0)
                    } else {
                        timestamp_offset(-4)
                    };
                    let exact_id = Uuid::from_u128(10_001);
                    let candidate_id = if unread_counterpart {
                        Uuid::from_u128(10_002)
                    } else {
                        exact_id
                    };
                    let boundary =
                        MessageKey::parse(&boundary_at, &Uuid::from_u128(50_000).to_string())?;
                    seed_read_points(&f, &session, channel, kind, boundary, exact_id)?;
                    let mut notif = row(
                        1,
                        channel,
                        Uuid::from_u128(4),
                        session.account().user(),
                        &candidate_at,
                    );
                    notif.message_id = Some(candidate_id.to_string());
                    f.server()
                        .set_notifications(session.account().user(), vec![notif])
                        .await?;
                    assert!(session.reading_state(channel).is_none());
                    let report = if push {
                        client.push_wake(session.account(), deadline()).await?
                    } else {
                        client.background_sync(deadline()).await?
                    };
                    assert_eq!(report.accounts[0].outcome, BackgroundOutcome::Complete);
                    assert_eq!(report.deliveries.len(), usize::from(unread_counterpart));
                    // Both suppressed eligible candidates and delivered ones claim the ledger.
                    assert_eq!(ledger_rows(&f)?, 1);
                    assert!(session.reading_state(channel).is_none());
                    hold.release();
                    Ok::<_, Box<dyn std::error::Error>>(())
                })??;
                f.finish()?;
            }
        }
    }
    Ok(())
}

#[test]
fn cold_background_and_push_still_suppress_on_backend_inbox_read_flag() -> Result {
    for push in [false, true] {
        let f = Fixture::new(papo_test_server::Config::default())?;
        f.run(async {
            let client = f.client().handle();
            let server = client
                .add_server(&f.server().base_url(), UnixMicros(0))
                .await?;
            let auth = client.auth(server)?;
            register(&auth, "cold-inbox-read").await?;
            let mut hold = f.server().hold_next_rest("GET", "/channels").await?;
            let session = login(&auth, "cold-inbox-read").await?;
            hold.wait_started().await?;
            let channel: Uuid = f
                .server()
                .create_channel("fixture")
                .await?
                .id
                .ok_or("channel")?
                .parse()?;
            client.push_wake(session.account(), deadline()).await?;
            let at = timestamp();
            let mut notif = row(
                1,
                channel,
                Uuid::from_u128(4),
                session.account().user(),
                &at,
            );
            notif.read = Some(true);
            f.server()
                .set_notifications(session.account().user(), vec![notif])
                .await?;
            let report = if push {
                client.push_wake(session.account(), deadline()).await?
            } else {
                client.background_sync(deadline()).await?
            };
            assert_eq!(report.accounts[0].outcome, BackgroundOutcome::Complete);
            assert!(report.deliveries.is_empty());
            assert_eq!(ledger_rows(&f)?, 1);
            hold.release();
            Ok::<_, Box<dyn std::error::Error>>(())
        })??;
        f.finish()?;
    }
    Ok(())
}
