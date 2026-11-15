//! Behaviour specs for remote reconciliation, offline restart, legacy ms anchors, deleted anchors and account isolation.

use super::{
    AUTHOR, CHANNEL, FIRST, Fixture, Result, channel, config, load_history, msg_id, msg_point,
    report_range, signed_in, signed_in_as, sqlite_catch_up_row, view,
};
use papo_api::HistoryQuery;
use papo_sdk::{ConnectionState, HistoryOrder, ReadingPosition, UnreadCount, VisibilityContinuity};

#[test]
fn remote_read_clears_badge_while_preserving_local_catch_up() -> Result {
    let fixture = Fixture::new(config(10))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let _keep_history_open = session.history(channel()).await?;
        let snap = load_history(&session, channel()).await?;
        report_range(&session, channel(), 0..=0, VisibilityContinuity::Continuous).await?;
        // Focus middle m5..=m6 -> catch-up [m2..=m4], local_point m1.
        report_range(
            &session,
            channel(),
            4..=5,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        assert!(view(&session, channel(), &snap)?.is_some());
        // Another device acknowledges m10 via server read mark + reconnect reconciliation.
        assert!(
            fixture
                .server()
                .advance_read_mark(
                    &session.account().user().to_string(),
                    CHANNEL,
                    &msg_id(&snap, 9).to_string(),
                    "2026-02-01T00:00:00Z",
                )
                .await?
        );
        let mut events = session.subscribe_reading();
        let before = session.generation();
        session.reconnect()?;
        let mut status = session.subscribe_connection();
        super::fixture::wait_for(&mut status, |s| {
            s.state == ConnectionState::Online && s.generation > before
        })
        .await?;
        let read = super::wait_read(&mut events, channel(), |s| {
            s.unread_count == UnreadCount::Exact(0)
        })
        .await?;
        assert_eq!(read.unread_count, UnreadCount::Exact(0));
        assert_eq!(read.uploaded_point.unwrap().message_id, msg_id(&snap, 9));
        // Local catch-up [m2..=m4] MUST survive remote head acknowledgement!
        let gap =
            view(&session, channel(), &snap)?.expect("remote read must preserve local catch-up");
        assert_eq!(gap.start_after, Some(msg_point(&snap, 0)));
        assert_eq!(gap.end_before, msg_point(&snap, 4));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn restart_restores_catch_up_and_head_independently_online_and_offline() -> Result {
    let mut fixture = Fixture::new(config(10))?;
    let (account, snap) = fixture.run(async {
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
        session
            .save_reading_position(
                channel(),
                Some(ReadingPosition {
                    message_id: msg_id(&snap, 4),
                    offset_units: -12,
                }),
            )
            .await?;
        Ok::<_, Box<dyn std::error::Error>>((session.account(), snap))
    })??;

    // Offline reopen (gateway disabled) must still restore durable catch-up, seen_to, and position.
    fixture.close_client()?;
    fixture.run_detached_offline_check(account, &snap)?;
    fixture.reopen()?;
    fixture.run(async {
        let session = fixture.client().handle().session(account)?;
        let gap = view(&session, channel(), &snap)?.unwrap();
        assert_eq!(gap.start_after, Some(msg_point(&snap, 0)));
        assert_eq!(gap.end_before, msg_point(&snap, 8));
        assert_eq!(
            sqlite_catch_up_row(fixture.profile.path(), channel())?,
            Some((Some(msg_point(&snap, 9)), true))
        );
        assert_eq!(
            session.reading_position(channel()),
            Some(ReadingPosition {
                message_id: msg_id(&snap, 4),
                offset_units: -12,
            })
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

trait OfflineCheck {
    fn run_detached_offline_check(
        &mut self,
        account: papo_sdk::AccountId,
        snap: &papo_sdk::HistorySnapshot,
    ) -> Result;
}

impl OfflineCheck for Fixture {
    fn run_detached_offline_check(
        &mut self,
        account: papo_sdk::AccountId,
        snap: &papo_sdk::HistorySnapshot,
    ) -> Result {
        self.reopen()?;
        self.run(async {
            self.server().set_gateway_accepting(false).await;
            let session = self.client().handle().session(account)?;
            assert!(
                session
                    .subscribe_catch_up()
                    .borrow()
                    .channels
                    .contains_key(&channel())
            );
            let gap = view(&session, channel(), snap)?.unwrap();
            assert_eq!(gap.start_after, Some(msg_point(snap, 0)));
            assert_eq!(gap.end_before, msg_point(snap, 8));
            self.server().set_gateway_accepting(true).await;
            Ok::<_, Box<dyn std::error::Error>>(())
        })?
    }
}

#[test]
fn account_isolation_and_removal_delete_catch_up_and_saved_positions() -> Result {
    let mut fixture = Fixture::new(config(10))?;
    let (alpha_id, beta_id, beta_msg) = fixture.run(async {
        let alpha = signed_in_as(&fixture, "alpha").await?;
        let snap = load_history(&alpha, channel()).await?;
        report_range(&alpha, channel(), 0..=0, VisibilityContinuity::Continuous).await?;
        report_range(
            &alpha,
            channel(),
            8..=9,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        let m5 = msg_id(&snap, 4);
        let beta_msg = msg_id(&snap, 2);
        alpha
            .save_reading_position(
                channel(),
                Some(ReadingPosition {
                    message_id: m5,
                    offset_units: 77,
                }),
            )
            .await?;
        assert!(view(&alpha, channel(), &snap)?.is_some());

        let beta = signed_in_as(&fixture, "beta").await?;
        assert!(beta.subscribe_catch_up().borrow().channels.is_empty());
        assert!(view(&beta, channel(), &snap)?.is_none());
        assert_eq!(beta.reading_position(channel()), None);
        beta.save_reading_position(
            channel(),
            Some(ReadingPosition {
                message_id: beta_msg,
                offset_units: 3,
            }),
        )
        .await?;

        // Sign out alpha & remove alpha account; beta must stay isolated and untouched.
        let alpha_id = alpha.account();
        let beta_id = beta.account();
        alpha.auth().logout().await?;
        fixture.client().handle().remove_account(alpha_id).await?;
        assert_eq!(
            beta.reading_position(channel()),
            Some(ReadingPosition {
                message_id: beta_msg,
                offset_units: 3,
            })
        );
        Ok::<_, Box<dyn std::error::Error>>((alpha_id, beta_id, beta_msg))
    })??;
    fixture.reopen()?;
    fixture.run(async {
        let beta = fixture.client().handle().session(beta_id)?;
        assert_eq!(
            beta.reading_position(channel()),
            Some(ReadingPosition {
                message_id: beta_msg,
                offset_units: 3,
            })
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    let db = rusqlite::Connection::open(fixture.profile.path().join("state.sqlite"))?;
    let alpha_account_bytes = alpha_id.user().as_bytes().to_vec();
    let catch_up_rows: i64 = db.query_row(
        "SELECT count(*) FROM catch_up_intervals WHERE user_id = ?1",
        [&alpha_account_bytes],
        |r| r.get(0),
    )?;
    let pos_rows: i64 = db.query_row(
        "SELECT count(*) FROM saved_reading_positions WHERE user_id = ?1",
        [&alpha_account_bytes],
        |r| r.get(0),
    )?;
    assert_eq!((catch_up_rows, pos_rows), (0, 0));
    let remaining_pos: i64 =
        db.query_row("SELECT count(*) FROM saved_reading_positions", [], |r| {
            r.get(0)
        })?;
    assert_eq!(remaining_pos, 1);
    fixture.finish()
}

#[test]
fn legacy_millisecond_anchor_and_deleted_anchor_advance_without_reopening_read_rows() -> Result {
    let mut cfg = config(0);
    cfg.history = vec![papo_test_server::HistorySeed {
        channel_id: CHANNEL.into(),
        author_id: AUTHOR.into(),
        first_message_id: FIRST.into(),
        count: 8,
        first_created_at: "2026-01-01T00:00:00.123100Z".into(),
        timestamp_step_us: 100,
        equal_timestamp_count: 1,
    }];
    let mut fixture = Fixture::new(cfg)?;
    let account = fixture.run(async {
        let session = signed_in(&fixture).await?;
        // Server carries a legacy millisecond-truncated read timestamp for m1.
        fixture
            .server()
            .set_read_mark(
                &session.account().user().to_string(),
                CHANNEL,
                FIRST,
                "2026-01-01T00:00:00.123Z",
            )
            .await?;
        let before = session.generation();
        session.reconnect()?;
        let mut status = session.subscribe_connection();
        super::fixture::wait_for(&mut status, |s| {
            s.state == ConnectionState::Online && s.generation > before
        })
        .await?;
        let snap = load_history(&session, channel()).await?;
        // Skip to head [m7..=m8] -> legacy m1 anchor must not reopen earlier rows, catch-up starts after m1.
        report_range(
            &session,
            channel(),
            6..=7,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        let gap = view(&session, channel(), &snap)?.unwrap();
        assert_eq!(gap.start_after.unwrap().message_id, msg_id(&snap, 0));

        // Now delete the anchor m1 on the server and reopen with a fresh partial tail/Asc fetch.
        assert!(fixture.server().delete_message(FIRST).await?);
        Ok::<_, Box<dyn std::error::Error>>(session.account())
    })??;
    fixture.reopen()?;
    fixture.run(async {
        let session = fixture.client().handle().session(account)?;
        super::wait_online(&session).await?;
        let history = session.history(channel()).await?;
        // Fetching Asc from start loads m2..=m5 after deleted m1 without wedging progress.
        history
            .page(HistoryQuery {
                cursor: None,
                order: HistoryOrder::Asc,
                limit: Some(4),
            })
            .await?;
        let first_page = history.subscribe().await?.snapshot;
        assert_ne!(first_page.messages[0].id.as_deref().unwrap(), FIRST);
        let after = first_page.messages.last().unwrap().cursor().unwrap();
        history
            .page(HistoryQuery {
                cursor: Some(&after),
                order: HistoryOrder::Asc,
                limit: Some(10),
            })
            .await?;
        let snap = history.subscribe().await?.snapshot;
        assert_eq!(snap.messages.len(), 7);
        report_range(&session, channel(), 0..=3, VisibilityContinuity::Continuous).await?;
        report_range(&session, channel(), 4..=6, VisibilityContinuity::Continuous).await?;
        assert!(view(&session, channel(), &snap)?.is_none());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn channel_removal_on_reconnect_deletes_catch_up_intervals_from_memory_and_sqlite() -> Result {
    let mut fixture = Fixture::new(config(10))?;
    let account = fixture.run(async {
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
        assert!(
            session
                .subscribe_catch_up()
                .borrow()
                .channels
                .contains_key(&channel())
        );
        assert!(view(&session, channel(), &snap)?.is_some());
        Ok::<_, Box<dyn std::error::Error>>(session.account())
    })??;
    fixture.reopen()?;
    fixture.run(async {
        let session = fixture.client().handle().session(account)?;
        assert!(
            session
                .subscribe_catch_up()
                .borrow()
                .channels
                .contains_key(&channel())
        );
        assert!(sqlite_catch_up_row(fixture.profile.path(), channel())?.is_some());
        // Remove the channel server-side while offline so no ChannelDelete event arrives;
        // reconnect reconciliation must purge catch_up_intervals too.
        assert!(fixture.server().delete_channel(CHANNEL).await);
        let mut cu_rx = session.subscribe_catch_up();
        super::fixture::online(&session).await?;
        super::wait_catch_up_absent(&mut cu_rx, channel()).await?;
        assert_eq!(
            sqlite_catch_up_row(fixture.profile.path(), channel())?,
            None
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.reopen()?;
    fixture.run(async {
        let session = fixture.client().handle().session(account)?;
        assert!(
            !session
                .subscribe_catch_up()
                .borrow()
                .channels
                .contains_key(&channel())
        );
        assert_eq!(
            sqlite_catch_up_row(fixture.profile.path(), channel())?,
            None
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
