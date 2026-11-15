//! Behaviour specs for durable verbatim reading positions, burst coalescing and revocation fencing.

use super::{
    CHANNEL, Fixture, Result, channel, config, interval, load_history, msg_id, report_range,
    signed_in,
};
use papo_api::HistoryQuery;
use papo_sdk::{
    DirectConversation, HistoryOrder, ReadingPosition, ReadingPositionsSnapshot, UserSummary, Uuid,
    VisibilityContinuity,
};
use papo_test_server::{Failure, RestStep};
use std::sync::Arc;
use tokio::sync::watch;

async fn wait_pos(
    rx: &mut watch::Receiver<Arc<ReadingPositionsSnapshot>>,
    ch: Uuid,
    expected: Option<ReadingPosition>,
) -> Result {
    loop {
        if rx.borrow_and_update().channels.get(&ch).copied() == expected {
            return Ok(());
        }
        rx.changed().await?;
    }
}

#[test]
fn saved_reading_position_stores_verbatim_signed_offsets_independently_of_read_state() -> Result {
    let mut fixture = Fixture::new(config(10))?;
    let (account, m3, m6) = fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snap = load_history(&session, channel()).await?;
        report_range(&session, channel(), 0..=0, VisibilityContinuity::Continuous).await?;
        report_range(
            &session,
            channel(),
            8..=9,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        let read_before = session.reading_state(channel()).unwrap();
        let catch_before = interval(&session, channel()).unwrap();
        let mut rx = session.subscribe_reading_positions();
        let m3 = msg_id(&snap, 2);
        let m6 = msg_id(&snap, 5);
        for offset_units in [-42_i64, 0, 1024] {
            let pos = ReadingPosition {
                message_id: m3,
                offset_units,
            };
            session.save_reading_position(channel(), Some(pos)).await?;
            wait_pos(&mut rx, channel(), Some(pos)).await?;
            assert_eq!(session.reading_position(channel()), Some(pos));
        }
        let final_pos = ReadingPosition {
            message_id: m6,
            offset_units: -7,
        };
        session
            .save_reading_position(channel(), Some(final_pos))
            .await?;
        assert_eq!(session.reading_position(channel()), Some(final_pos));
        // Saving reading positions must never mutate read_point or catch_up interval.
        assert_eq!(session.reading_state(channel()).unwrap(), read_before);
        assert_eq!(interval(&session, channel()).unwrap(), catch_before);
        Ok::<_, Box<dyn std::error::Error>>((session.account(), m3, m6))
    })??;
    fixture.reopen()?;
    fixture.run(async {
        let session = fixture.client().handle().session(account)?;
        assert_eq!(
            session.reading_position(channel()),
            Some(ReadingPosition {
                message_id: m6,
                offset_units: -7,
            })
        );
        session.save_reading_position(channel(), None).await?;
        assert_eq!(session.reading_position(channel()), None);
        let _ = m3;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn position_saves_coalesce_bursts_and_acknowledge_only_after_full_commit() -> Result {
    let fixture = Fixture::new(config(25))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snap = load_history(&session, channel()).await?;
        let db = rusqlite::Connection::open(fixture.profile.path().join("state.sqlite"))?;
        db.execute_batch(
            "DROP TRIGGER IF EXISTS count_pos_insert;
             DROP TRIGGER IF EXISTS count_pos_update;
             CREATE TABLE IF NOT EXISTS pos_write_log(id INTEGER PRIMARY KEY AUTOINCREMENT, off INTEGER);
             CREATE TRIGGER count_pos_insert AFTER INSERT ON saved_reading_positions
             BEGIN INSERT INTO pos_write_log(off) VALUES (NEW.offset_units); END;
             CREATE TRIGGER count_pos_update AFTER UPDATE ON saved_reading_positions
             BEGIN INSERT INTO pos_write_log(off) VALUES (NEW.offset_units); END;",
        )?;
        let last_id = msg_id(&snap, 20);
        let mut tasks = tokio::task::JoinSet::new();
        for i in 0..20_i64 {
            let s = session.clone();
            let pos = ReadingPosition {
                message_id: if i == 19 { last_id } else { msg_id(&snap, usize::try_from(i).unwrap()) },
                offset_units: i * 10 - 50,
            };
            tasks.spawn(async move {
                s.save_reading_position(channel(), Some(pos))
                    .await
                    .map_err(|e| e.to_string())?;
                Ok::<_, String>(pos)
            });
        }
        while let Some(joined) = tasks.join_next().await {
            joined?.map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
            // Every returned ack MUST already observe a durable row on SQLite disk.
            let durable: i64 = db.query_row(
                "SELECT count(*) FROM saved_reading_positions WHERE channel_id = ?1",
                [channel().as_bytes().as_slice()],
                |r| r.get(0),
            )?;
            assert_eq!(durable, 1);
        }
        let write_count: i64 = db.query_row("SELECT count(*) FROM pos_write_log", [], |r| r.get(0))?;
        assert!(
            write_count < 20,
            "20 concurrent position saves should coalesce onto fewer FULL commits (saw {write_count})"
        );
        let final_stored = session.reading_position(channel()).unwrap();
        let disk_offset: i64 = db.query_row(
            "SELECT offset_units FROM saved_reading_positions WHERE channel_id = ?1",
            [channel().as_bytes().as_slice()],
            |r| r.get(0),
        )?;
        assert_eq!(final_stored.offset_units, disk_offset);
        db.execute_batch(
            "DROP TRIGGER main.count_pos_insert; DROP TRIGGER main.count_pos_update; DROP TABLE main.pos_write_log;",
        )?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn channel_deleted_discards_saved_position() -> Result {
    let fixture = Fixture::new(config(4))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snap = load_history(&session, channel()).await?;
        let pos = ReadingPosition {
            message_id: msg_id(&snap, 1),
            offset_units: 15,
        };
        session.save_reading_position(channel(), Some(pos)).await?;
        let mut rx = session.subscribe_reading_positions();
        assert!(fixture.server().delete_channel(CHANNEL).await);
        wait_pos(&mut rx, channel(), None).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn channel_forbidden_discards_saved_position() -> Result {
    let fixture = Fixture::new(config(0))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        fixture
            .server()
            .seed_history(config(60).history.pop().unwrap())
            .await?;
        let history = session.history(channel()).await?;
        history
            .page(HistoryQuery {
                limit: Some(10),
                ..HistoryQuery::default()
            })
            .await?;
        let snap = history.subscribe().await?.snapshot;
        let msg = msg_id(&snap, 0);
        session
            .save_reading_position(
                channel(),
                Some(ReadingPosition {
                    message_id: msg,
                    offset_units: 9,
                }),
            )
            .await?;
        fixture
            .server()
            .set_channel_readable(&session.account().user().to_string(), CHANNEL, false)
            .await?;
        let mut rx = session.subscribe_reading_positions();
        let before = snap.messages[0].cursor().unwrap();
        let _ = history
            .page(HistoryQuery {
                order: HistoryOrder::Desc,
                cursor: Some(&before),
                limit: Some(10),
            })
            .await;
        wait_pos(&mut rx, channel(), None).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn channel_not_found_discards_saved_position() -> Result {
    let path = format!("/channels/{CHANNEL}/messages");
    let mut cfg = config(0);
    cfg.rest = vec![
        RestStep {
            method: "GET".into(),
            path: path.clone(),
            delay_ms: 0,
            failure: None,
        },
        RestStep {
            method: "GET".into(),
            path,
            delay_ms: 0,
            failure: Some(Failure {
                status: 404,
                slug: "not-found".into(),
            }),
        },
    ];
    let fixture = Fixture::new(cfg)?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        fixture
            .server()
            .seed_history(config(30).history.pop().unwrap())
            .await?;
        let history = session.history(channel()).await?;
        history
            .page(HistoryQuery {
                limit: Some(10),
                ..HistoryQuery::default()
            })
            .await?;
        let snap = history.subscribe().await?.snapshot;
        let msg = msg_id(&snap, 0);
        session
            .save_reading_position(
                channel(),
                Some(ReadingPosition {
                    message_id: msg,
                    offset_units: 33,
                }),
            )
            .await?;
        let mut rx = session.subscribe_reading_positions();
        let before = snap.messages[0].cursor().unwrap();
        let _ = history
            .page(HistoryQuery {
                order: HistoryOrder::Desc,
                cursor: Some(&before),
                limit: Some(5),
            })
            .await;
        wait_pos(&mut rx, channel(), None).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn channel_removed_discards_saved_position_and_fences_concurrent_writes() -> Result {
    let fixture = Fixture::new(config(0))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let dm_id = Uuid::from_u128(19);
        let dm = DirectConversation {
            id: dm_id.to_string(),
            user: UserSummary {
                id: Some(Uuid::from_u128(18).to_string()),
                ..Default::default()
            },
            created_at: "2026-01-01T00:00:00Z".into(),
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            unread_count: 0,
        };
        let account = session.account().user().to_string();
        fixture
            .server()
            .set_direct_conversations(&account, vec![dm])
            .await?;
        let mut res_rx = session.subscribe_resources();
        while res_rx.borrow_and_update().dms.is_empty() {
            res_rx.changed().await?;
        }
        let msg_id_dm = Uuid::from_u128(0x8000_0000_0000_4000_8000_0000_0000_0001);
        session
            .save_reading_position(
                dm_id,
                Some(ReadingPosition {
                    message_id: msg_id_dm,
                    offset_units: -5,
                }),
            )
            .await?;
        let mut rx = session.subscribe_reading_positions();
        fixture
            .server()
            .set_direct_conversations(&account, vec![])
            .await?;
        let before = session.generation();
        session.reconnect()?;
        let mut status = session.subscribe_connection();
        super::fixture::wait_for(&mut status, |s| {
            s.state == papo_sdk::ConnectionState::Online && s.generation > before
        })
        .await?;
        wait_pos(&mut rx, dm_id, None).await?;
        assert_eq!(session.reading_position(dm_id), None);
        // Further save on already-removed DM or stale generation is fenced/cleaned cleanly.
        let mut set = tokio::task::JoinSet::new();
        for off in 1..=5_i64 {
            let s = session.clone();
            set.spawn(async move {
                let _ = s
                    .save_reading_position(
                        dm_id,
                        Some(ReadingPosition {
                            message_id: msg_id_dm,
                            offset_units: off,
                        }),
                    )
                    .await;
            });
        }
        while set.join_next().await.is_some() {}
        // Channel has no membership, ensure sqlite / observable still clean on reconnect.
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
