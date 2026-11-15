//! Behaviour specs for catch-up interval lifecycle, continuity, replacement, own messages and clearing.

use super::{
    CHANNEL, Fixture, Result, channel, config, interval, load_history, msg_id, msg_point,
    report_range, report_with, signed_in, synthetic_message,
};
use papo_api::HistoryQuery;
use papo_sdk::{UnreadCount, VisibilityContinuity};

#[test]
fn live_head_acknowledgement_clears_unread_while_preserving_catch_up_and_closing_from_both_ends()
-> Result {
    let mut fixture = Fixture::new(config(10))?;
    let expected_seen = fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snap = load_history(&session, channel()).await?;
        report_range(&session, channel(), 0..=0, VisibilityContinuity::Continuous).await?;
        assert_eq!(
            session.reading_state(channel()).unwrap().unread_count,
            UnreadCount::Exact(9)
        );
        // Jump past [m2..=m8] straight to [m9..=m10] -> badge clears, catch-up stays [m2..=m8].
        report_range(
            &session,
            channel(),
            8..=9,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        let read = session.reading_state(channel()).unwrap();
        assert_eq!(read.unread_count, UnreadCount::Exact(0));
        assert_eq!(read.local_point.unwrap().message_id, msg_id(&snap, 9));
        let gap = interval(&session, channel()).expect("catch-up must remain after live-head skip");
        assert_eq!(gap.start_after, Some(msg_point(&snap, 0)));
        assert_eq!(gap.end_before, msg_point(&snap, 8));

        let view = session.catch_up_view(channel(), &snap)?;
        assert_eq!(view.start_target, Some(msg_id(&snap, 1)));

        // Consume top [m2..=m3] and bottom [m7..=m8] continuously.
        report_range(
            &session,
            channel(),
            1..=2,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        let mid = interval(&session, channel()).expect("middle must remain");
        assert_eq!(mid.start_after, Some(msg_point(&snap, 2)));
        assert_eq!(mid.end_before, msg_point(&snap, 8));

        report_range(&session, channel(), 7..=8, VisibilityContinuity::Continuous).await?;
        report_range(&session, channel(), 6..=7, VisibilityContinuity::Continuous).await?;
        let mid = interval(&session, channel()).expect("m4..=m6 must remain");
        assert_eq!(mid.start_after, Some(msg_point(&snap, 2)));
        assert_eq!(mid.end_before, msg_point(&snap, 6));

        // Finish the remaining middle [m4..=m6] continuously -> interval retires cleanly.
        report_range(&session, channel(), 3..=6, VisibilityContinuity::Continuous).await?;
        assert!(interval(&session, channel()).is_none());
        let seen = msg_point(&snap, 9);
        assert_eq!(
            session.catch_up_state(channel()).unwrap().seen_to,
            Some(seen)
        );
        Ok::<_, Box<dyn std::error::Error>>(seen)
    })??;
    fixture.reopen()?;
    fixture.run(async {
        let account = fixture
            .client()
            .handle()
            .subscribe_registry()
            .borrow()
            .accounts[0]
            .id;
        let session = fixture.client().handle().session(account)?;
        assert!(interval(&session, channel()).is_none());
        assert_eq!(
            session.catch_up_state(channel()).unwrap().seen_to,
            Some(expected_seen)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn new_arrival_behind_focused_viewport_stays_unread_and_reaching_window_end_never_acks_head()
-> Result {
    let fixture = Fixture::new(config(0))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        fixture
            .server()
            .seed_history(config(12).history.pop().unwrap())
            .await?;
        let history = session.history(channel()).await?;
        // Load only the older 6 messages so history coverage does not include the live head (m12).
        history
            .page(HistoryQuery {
                cursor: None,
                order: papo_sdk::HistoryOrder::Asc,
                limit: Some(6),
            })
            .await?;
        let snap = history.subscribe().await?.snapshot;
        assert_eq!(snap.messages.len(), 6);
        report_range(&session, channel(), 0..=0, VisibilityContinuity::Continuous).await?;
        report_range(
            &session,
            channel(),
            4..=5,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        // Reaching the end of the 6-message window advances local_point only to loaded m6, NEVER live head m12.
        let partial_read = session.reading_state(channel()).unwrap();
        assert_eq!(
            partial_read.local_point.unwrap().message_id,
            msg_id(&snap, 5)
        );
        assert_ne!(partial_read.unread_count, UnreadCount::Exact(0));
        // Load all 12, visit m11..=m12 (keeping [m2..=m4] in catch-up), then jump back and emit m13.
        let full = load_history(&session, channel()).await?;
        report_range(
            &session,
            channel(),
            10..=11,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        assert!(interval(&session, channel()).is_some());
        let mut events = session.subscribe_reading();
        let m13 = synthetic_message(CHANNEL, super::AUTHOR, 900, "2026-01-01T00:05:00.000000Z");
        fixture.server().publish_message(m13).await?;
        super::wait_read(&mut events, channel(), |s| {
            s.unread_count != UnreadCount::Exact(0)
        })
        .await?;
        let after = session.reading_state(channel()).unwrap();
        assert_eq!(after.unread_count, UnreadCount::Exact(1));
        assert_eq!(after.local_point.unwrap().message_id, msg_id(&full, 11));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn skipping_newer_block_replaces_older_interval_using_prev_head_while_exposed_arrivals_preserve_it()
-> Result {
    let fixture = Fixture::new(config(14))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snap = load_history(&session, channel()).await?;
        // Establish read m1, then skip to [m6..=m7] -> catch-up [m2..=m5], prev_head = m7.
        report_range(&session, channel(), 0..=0, VisibilityContinuity::Continuous).await?;
        report_range(
            &session,
            channel(),
            5..=6,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        let first = interval(&session, channel()).unwrap();
        assert_eq!(first.start_after, Some(msg_point(&snap, 0)));
        assert_eq!(first.end_before, msg_point(&snap, 5));

        // Skip newer block m8..=m11 to [m12..=m13] -> replaces older interval with [m8..=m11].
        report_range(
            &session,
            channel(),
            11..=12,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        let second = interval(&session, channel()).unwrap();
        assert_eq!(second.start_after, Some(msg_point(&snap, 6)));
        assert_eq!(second.end_before, msg_point(&snap, 11));

        // Live arrivals exposed immediately at head [m13..=m15] preserve the older catch-up [m8..=m11].
        let mut events = session.subscribe_events();
        let m15 = synthetic_message(CHANNEL, super::AUTHOR, 901, "2026-01-01T00:06:00.000000Z");
        fixture.server().publish_message(m15).await?;
        while events.next().await?.value.kind() != "message" {}
        report_range(
            &session,
            channel(),
            12..=14,
            VisibilityContinuity::Continuous,
        )
        .await?;
        let preserved = interval(&session, channel()).unwrap();
        assert_eq!(preserved.start_after, Some(msg_point(&snap, 6)));
        assert_eq!(preserved.end_before, msg_point(&snap, 11));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn middle_context_page_never_consumes_unloaded_gaps_and_own_messages_never_hold_catch_up() -> Result
{
    let fixture = Fixture::new(config(0))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let me = session.account().user().to_string();
        let msgs = mixed_author_seed(&me);
        fixture.server().seed_messages(CHANNEL, msgs).await?;
        let snap = load_history(&session, channel()).await?;
        // Wait: m1 (idx 0) foreign, m2..=m7 (idx 1..=6) foreign, m8..=m9 (idx 7..=8) OWN, m10 (idx 9) OWN.
        // Report m1 then skip to [m5..=m6] -> catch-up [m2..=m4].
        report_range(&session, channel(), 0..=0, VisibilityContinuity::Continuous).await?;
        report_range(
            &session,
            channel(),
            4..=5,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        let gap = interval(&session, channel()).unwrap();
        assert_eq!(gap.start_after, Some(msg_point(&snap, 0)));
        assert_eq!(gap.end_before, msg_point(&snap, 4));
        // Middle span [m3] via Discontinuous keeps start_after=m1 and records covered span without closing m2/m4.
        report_range(
            &session,
            channel(),
            2..=2,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        let mid = interval(&session, channel()).unwrap();
        assert_eq!(mid.start_after, Some(msg_point(&snap, 0)));
        assert_eq!(mid.end_before, msg_point(&snap, 4));
        assert_eq!(mid.covered_spans.len(), 1);
        // Viewing m2..=m4 closes through the covered span; own messages m8..=m10 skipped later never open catch-up.
        report_range(&session, channel(), 1..=1, VisibilityContinuity::Continuous).await?;
        report_range(
            &session,
            channel(),
            3..=3,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        assert!(interval(&session, channel()).is_none());
        // Skip over OWN messages m8..=m9 straight to m10 -> zero catch-up!
        report_range(
            &session,
            channel(),
            9..=9,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        assert!(interval(&session, channel()).is_none());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

fn mixed_author_seed(me: &str) -> Vec<papo_api::wire::Message> {
    let mut msgs = papo_test_server::HistorySeed {
        channel_id: CHANNEL.into(),
        author_id: super::AUTHOR.into(),
        first_message_id: super::FIRST.into(),
        count: 10,
        first_created_at: "2026-01-01T00:00:00.123450Z".into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 1,
    }
    .messages()
    .unwrap();
    for msg in msgs.iter_mut().skip(6) {
        msg.author_id = Some(me.to_owned());
    }
    msgs
}

#[test]
fn multi_window_observers_isolate_continuity_and_clear_catch_up_persists_durably() -> Result {
    let mut fixture = Fixture::new(config(10))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let snap = load_history(&session, channel()).await?;
        let live_win = session.reading_observer();
        let focused_win = session.reading_observer();

        report_with(
            &live_win,
            &session,
            channel(),
            0..=0,
            VisibilityContinuity::Continuous,
        )
        .await?;
        // Live window skips to [m9..=m10] -> catch-up [m2..=m8].
        report_with(
            &live_win,
            &session,
            channel(),
            8..=9,
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        // Focused window views [m4..=m5] with Continuous -> must NOT cross-bridge [m5..=m9] from live_win!
        report_with(
            &focused_win,
            &session,
            channel(),
            3..=4,
            VisibilityContinuity::Continuous,
        )
        .await?;
        let gap = interval(&session, channel()).unwrap();
        assert_eq!(gap.start_after, Some(msg_point(&snap, 0)));
        assert_eq!(gap.end_before, msg_point(&snap, 8));

        // Deliberate clear removes the interval durably and advances seen_to.
        session.clear_catch_up(channel()).await?;
        assert!(interval(&session, channel()).is_none());
        assert_eq!(
            session.catch_up_state(channel()).unwrap().seen_to,
            Some(msg_point(&snap, 9))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.reopen()?;
    fixture.run(async {
        let account = fixture
            .client()
            .handle()
            .subscribe_registry()
            .borrow()
            .accounts[0]
            .id;
        let session = fixture.client().handle().session(account)?;
        assert!(interval(&session, channel()).is_none());
        assert!(session.catch_up_state(channel()).unwrap().seen_to.is_some());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn asc_page_without_cursor_marks_oldest_complete_even_with_more_pages() -> Result {
    let fixture = Fixture::new(config(6))?;
    fixture.run(async {
        let session = signed_in(&fixture).await?;
        let history = session.history(channel()).await?;
        let page = history
            .page(papo_api::HistoryQuery {
                cursor: None,
                order: papo_sdk::HistoryOrder::Asc,
                limit: Some(2),
            })
            .await?;
        assert!(page.has_more);
        let snap = history.subscribe().await?.snapshot;
        assert!(!snap.chunks.is_empty());
        assert!(snap.chunks[0].oldest_complete);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
