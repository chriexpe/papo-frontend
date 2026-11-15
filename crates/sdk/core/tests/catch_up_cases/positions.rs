//! Behaviour specs for durable verbatim reading positions, burst coalescing and revocation fencing.

use super::{
    CHANNEL, Fixture, Result, channel, config, load_history, msg_id, report_range, signed_in,
    wait_pos,
};
use papo_api::HistoryQuery;
use papo_sdk::{HistoryOrder, ReadingPosition, VisibilityContinuity};
use papo_test_server::{Failure, RestStep};

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
        let catch_before = super::view(&session, channel(), &snap)?.unwrap();
        let catch_row_before = super::sqlite_catch_up_row(fixture.profile.path(), channel())?;
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
        let catch_after = super::view(&session, channel(), &snap)?.unwrap();
        assert_eq!(
            (
                catch_after.start_after,
                catch_after.end_before,
                catch_after.needs_history
            ),
            (
                catch_before.start_after,
                catch_before.end_before,
                catch_before.needs_history
            )
        );
        assert_eq!(
            super::sqlite_catch_up_row(fixture.profile.path(), channel())?,
            catch_row_before
        );
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
        let observer = session.reading_observer();
        observer
            .report_visible(
                channel(),
                snap.generation,
                snap.version,
                &[msg_id(&snap, 0)],
                papo_sdk::VisibilityContinuity::Continuous,
            )
            .await?;
        observer
            .report_visible(
                channel(),
                snap.generation,
                snap.version,
                &[msg_id(&snap, 3)],
                papo_sdk::VisibilityContinuity::Discontinuous,
            )
            .await?;
        let mut rx = session.subscribe_reading_positions();
        let mut cu_rx = session.subscribe_catch_up();
        assert!(cu_rx.borrow().channels.contains_key(&channel()));
        assert!(fixture.server().delete_channel(CHANNEL).await);
        wait_pos(&mut rx, channel(), None).await?;
        super::wait_catch_up_absent(&mut cu_rx, channel()).await?;
        assert_eq!(
            super::sqlite_catch_up_row(fixture.profile.path(), channel())?,
            None
        );
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
        let observer = session.reading_observer();
        super::report_with(
            &observer,
            &session,
            channel(),
            0..=0,
            papo_sdk::VisibilityContinuity::Continuous,
        )
        .await?;
        super::report_with(
            &observer,
            &session,
            channel(),
            9..=9,
            papo_sdk::VisibilityContinuity::Discontinuous,
        )
        .await?;
        fixture
            .server()
            .set_channel_readable(&session.account().user().to_string(), CHANNEL, false)
            .await?;
        let mut rx = session.subscribe_reading_positions();
        let mut cu_rx = session.subscribe_catch_up();
        assert!(cu_rx.borrow().channels.contains_key(&channel()));
        let before = snap.messages[0].cursor().unwrap();
        let _ = history
            .page(HistoryQuery {
                order: HistoryOrder::Desc,
                cursor: Some(&before),
                limit: Some(10),
            })
            .await;
        wait_pos(&mut rx, channel(), None).await?;
        super::wait_catch_up_absent(&mut cu_rx, channel()).await?;
        assert_eq!(
            super::sqlite_catch_up_row(fixture.profile.path(), channel())?,
            None
        );
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
