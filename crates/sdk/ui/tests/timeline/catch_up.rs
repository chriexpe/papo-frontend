//! Public Session 13b Timeline integration specs: hourglass seek/cancel and shared reading overlay helpers.

use super::support::{CHANNEL, Fixture, Result, channel, config, ids};
use futures_util::StreamExt;
use papo_sdk::{Message, Uuid, VisibilityContinuity};
use papo_sdk_ui::{Timeline, TimelineError, TimelineReadingEvent, TimelineReadingState};
use papo_test_server::HistorySeed;

pub(super) fn extra_message(seq: u128, author: &str, created_at: &str) -> Message {
    let mut msg = HistorySeed {
        channel_id: CHANNEL.into(),
        author_id: author.into(),
        first_message_id: Uuid::from_u128(0x8800_0000_0000_4000_8000_0000_0000_0000 + seq)
            .to_string(),
        count: 1,
        first_created_at: created_at.into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 1,
    }
    .messages()
    .unwrap()
    .pop()
    .unwrap();
    msg.author_id = Some(author.into());
    msg
}

pub(super) async fn wait_overlay(
    rx: &mut papo_sdk_ui::TimelineReadingSubscription,
    predicate: impl Fn(&TimelineReadingState) -> bool,
) -> Result<TimelineReadingState> {
    if predicate(&rx.state) {
        return Ok(rx.state.clone());
    }
    loop {
        match rx
            .stream
            .next()
            .await
            .ok_or("reading overlay stream ended early")?
        {
            TimelineReadingEvent::Update(next) => {
                rx.state = next.clone();
                if predicate(&next) {
                    return Ok(next);
                }
            }
            TimelineReadingEvent::Closed(reason) => {
                return Err(format!("reading overlay closed cleanly: {reason:?}").into());
            }
            _ => return Err("unknown reading overlay event".into()),
        }
    }
}

pub(super) async fn report_visible_current(
    timeline: &Timeline,
    pick: impl Fn(&[Uuid]) -> Vec<Uuid>,
    continuity: VisibilityContinuity,
) -> Result {
    for _ in 0..32 {
        let sub = timeline.subscribe()?;
        let picked = pick(&ids(&sub));
        match timeline
            .report_visible(sub.version, sub.generation, &picked, continuity)
            .await
        {
            Ok(()) => return Ok(()),
            Err(
                TimelineError::StaleSnapshot
                | TimelineError::History(papo_sdk::HistoryError::StaleSnapshot)
                | TimelineError::Reading(papo_sdk::ReadingError::History(
                    papo_sdk::HistoryError::StaleSnapshot,
                )),
            ) => tokio::task::yield_now().await,
            Err(other) => return Err(other.into()),
        }
    }
    Err("report_visible remained stale".into())
}

fn channel_seed(count: usize) -> Vec<Message> {
    HistorySeed {
        channel_id: CHANNEL.into(),
        author_id: "20000000-0000-4000-8000-000000000001".into(),
        first_message_id: "70000000-0000-4000-8000-000000000001".into(),
        count,
        first_created_at: "2026-01-01T01:00:00.123456+01:00".into(),
        timestamp_step_us: 10,
        equal_timestamp_count: 1,
    }
    .messages()
    .unwrap()
}

pub(super) async fn reconnect_online(session: &papo_sdk::Session) -> Result {
    let before = session.generation();
    session.reconnect()?;
    let mut status = session.subscribe_connection();
    loop {
        let s = *status.borrow_and_update();
        if s.state == papo_sdk::ConnectionState::Online && s.generation > before {
            return Ok(());
        }
        status.changed().await?;
    }
}

#[test]
fn catch_up_loads_the_missing_prefix_and_never_navigates_to_the_read_anchor() -> Result {
    let fixture = Fixture::new(0, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("ui-hourglass").await?;
        let msgs = channel_seed(10);
        let m1_id = msgs[0].id.clone().unwrap();
        let m1_at = msgs[0].created_at.clone().unwrap();
        let m2_uuid: Uuid = msgs[1].id.as_deref().unwrap().parse()?;
        let m1_uuid: Uuid = m1_id.parse()?;

        // Mark m1 read while it is the sole channel head so unread=0 keeps history unloaded,
        // then silently seed m2..=m10 on the server.
        fixture
            .server()
            .seed_messages(CHANNEL, msgs[..1].to_vec())
            .await?;
        fixture
            .server()
            .set_read_mark(
                &session.account().user().to_string(),
                CHANNEL,
                &m1_id,
                &m1_at,
            )
            .await?;
        reconnect_online(&session).await?;
        fixture
            .server()
            .seed_messages(CHANNEL, msgs[1..].to_vec())
            .await?;

        let mut cfg = config();
        cfg.page_size = 2;
        let live = Timeline::live(session.clone(), channel(), cfg.clone()).await?;
        let live_sub = live.subscribe()?;
        assert_eq!(ids(&live_sub).len(), 2);
        report_visible_current(
            &live,
            <[papo_sdk::Uuid]>::to_vec,
            VisibilityContinuity::Continuous,
        )
        .await?;

        let state = live.reading_state()?;
        assert!(state.catch_up.available);
        assert_eq!(state.catch_up.start_after_id, Some(m1_uuid));
        assert!(state.catch_up.needs_history);
        assert_eq!(state.catch_up.start_target_id, None);

        // Even if the old read anchor m1 is deleted on the server before seeking,
        // hourglass pagination starts strictly after m1's key and focuses m2 (never m1).
        assert!(fixture.server().delete_message(&m1_id).await?);
        let seek = live.seek_catch_up(cfg)?;
        assert!(live.reading_state()?.catch_up.seeking);
        let focused = seek.finish().await?;
        let focused_sub = focused.subscribe()?;
        let focused_ids = ids(&focused_sub);
        assert_eq!(focused_ids.first().copied(), Some(m2_uuid));
        assert!(!focused_ids.contains(&m1_uuid));

        let settled = live.reading_state()?;
        assert!(!settled.catch_up.seeking);
        assert_eq!(settled.catch_up.start_target_id, Some(m2_uuid));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn leaving_a_catch_up_seek_cancels_future_pagination_without_erasing_the_block() -> Result {
    let fixture = Fixture::new(0, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("ui-seek-cancel").await?;
        let msgs = channel_seed(20);
        let m1_id = msgs[0].id.clone().unwrap();
        let m1_at = msgs[0].created_at.clone().unwrap();
        fixture
            .server()
            .seed_messages(CHANNEL, msgs[..1].to_vec())
            .await?;
        fixture
            .server()
            .set_read_mark(
                &session.account().user().to_string(),
                CHANNEL,
                &m1_id,
                &m1_at,
            )
            .await?;
        reconnect_online(&session).await?;
        fixture
            .server()
            .seed_messages(CHANNEL, msgs[1..].to_vec())
            .await?;

        let mut cfg = config();
        cfg.page_size = 2;
        let live = Timeline::live(session.clone(), channel(), cfg.clone()).await?;
        report_visible_current(
            &live,
            <[papo_sdk::Uuid]>::to_vec,
            VisibilityContinuity::Continuous,
        )
        .await?;

        let path = format!("/channels/{CHANNEL}/messages");
        let mut hold = fixture.server().hold_next_rest("GET", &path).await?;
        let seek = live.seek_catch_up(cfg.clone())?;
        assert!(live.reading_state()?.catch_up.seeking);

        let before_count = fixture.server().read_request_counts().await.history_pages;
        tokio::select! {
            biased;
            () = async {
                hold.wait_started().await.unwrap();
            } => {}
            res = seek.finish() => {
                return Err(format!("seek finished before cancel (ok={})", res.is_ok()).into());
            }
        }
        // Dropping the `seek.finish()` future fires `SeekCancel::drop` + `SeekingGuard::drop`.
        let cancelled_at = fixture.server().read_request_counts().await.history_pages;
        hold.release();
        tokio::task::yield_now().await;
        let after_count = fixture.server().read_request_counts().await.history_pages;
        assert!(cancelled_at > before_count);
        assert_eq!(
            after_count, cancelled_at,
            "no further Asc pages may run after cancellation"
        );

        let state_after_cancel = live.reading_state()?;
        assert!(!state_after_cancel.catch_up.seeking);
        assert!(state_after_cancel.catch_up.available);

        // Calling `.cancel()` explicitly before `.finish()` also returns `TimelineError::Cancelled` without erasing catch-up.
        let seek2 = live.seek_catch_up(cfg)?;
        seek2.cancel();
        assert!(matches!(
            seek2.finish().await,
            Err(TimelineError::Cancelled)
        ));
        let state_after_explicit = live.reading_state()?;
        assert!(!state_after_explicit.catch_up.seeking);
        assert!(state_after_explicit.catch_up.available);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
