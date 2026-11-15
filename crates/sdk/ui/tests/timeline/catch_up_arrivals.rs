//! Public Session 13b Timeline specs: behind-head live arrivals, deletes, two-stop jumps and zero item diffs.

use super::catch_up::{extra_message, reconnect_online, report_visible_current, wait_overlay};
use super::support::{CHANNEL, Fixture, Result, channel, config, ids, until};
use futures_util::{FutureExt, StreamExt};
use papo_sdk::{ReadingPosition, Uuid, VisibilityContinuity};
use papo_sdk_ui::{Timeline, TimelineEvent};

#[test]
fn behind_head_live_arrivals_queue_own_messages_deletes_and_two_stop_navigation() -> Result {
    let fixture = Fixture::new(10, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("ui-arrivals").await?;
        let mut cfg = config();
        cfg.page_size = 10;
        let live = Timeline::live(session.clone(), channel(), cfg.clone()).await?;
        let mut live_sub = live.subscribe()?;
        // Mark m1 read, then jump to [m9..=m10] -> catch-up [m2..=m8], local_point = m10.
        report_visible_current(&live, |v| vec![v[0]], VisibilityContinuity::Continuous).await?;
        report_visible_current(
            &live,
            |v| v[8..=9].to_vec(),
            VisibilityContinuity::Discontinuous,
        )
        .await?;

        let m3_cursor = fixture.messages[2].cursor().unwrap();
        let mut focus_cfg = config();
        focus_cfg.page_size = 4;
        let focused = Timeline::focused(session.clone(), channel(), m3_cursor, focus_cfg).await?;
        let mut overlay = focused.subscribe_reading()?;
        assert_eq!(overlay.state.arrivals.count, 0);

        let foreign_author = fixture.messages[0].author_id.as_deref().unwrap().to_owned();
        let own_author = session.account().user().to_string();
        let m11 = extra_message(11, &foreign_author, "2026-01-01T02:00:00.000001Z");
        let m12_own = extra_message(12, &own_author, "2026-01-01T02:00:00.000002Z");
        let m13 = extra_message(13, &foreign_author, "2026-01-01T02:00:00.000003Z");
        let m11_id = m11.id.clone().unwrap();
        let m13_id = m13.id.clone().unwrap();

        fixture.server().publish_message(m11).await?;
        fixture.server().publish_message(m12_own).await?;
        fixture.server().publish_message(m13).await?;

        // Only the 2 foreign arrivals increment `arrivals.count`; own message m12 is excluded.
        let s = wait_overlay(&mut overlay, |st| st.arrivals.count == 2).await?;
        assert_eq!(
            s.arrivals.oldest_unseen.as_ref().map(|c| c.id.as_str()),
            Some(m11_id.as_str())
        );
        assert_eq!(
            s.arrivals.live_head.as_ref().map(|c| c.id.as_str()),
            Some(m13_id.as_str())
        );

        // Deleting m11 decrements count and advances first stop (`oldest_unseen`) to m13.
        assert!(fixture.server().delete_message(&m11_id).await?);
        let s = wait_overlay(&mut overlay, |st| st.arrivals.count == 1).await?;
        assert_eq!(
            s.arrivals.oldest_unseen.as_ref().map(|c| c.id.as_str()),
            Some(m13_id.as_str())
        );

        // Add m14 -> count=2 (`oldest_unseen=m13`, `live_head=m14`), jump to first stop (m13), then second stop (m14).
        let m14 = extra_message(14, &foreign_author, "2026-01-01T02:00:00.000004Z");
        let m14_id = m14.id.clone().unwrap();
        fixture.server().publish_message(m14).await?;
        let s = wait_overlay(&mut overlay, |st| st.arrivals.count == 2).await?;
        assert_eq!(
            s.arrivals.oldest_unseen.as_ref().map(|c| c.id.as_str()),
            Some(m13_id.as_str())
        );
        assert_eq!(
            s.arrivals.live_head.as_ref().map(|c| c.id.as_str()),
            Some(m14_id.as_str())
        );

        let m13_uuid: Uuid = m13_id.parse()?;
        let m14_uuid: Uuid = m14_id.parse()?;
        until(&mut live_sub, |s| ids(s).contains(&m14_uuid)).await?;
        report_visible_current(&live, |_| vec![m13_uuid], VisibilityContinuity::Continuous).await?;
        let s = wait_overlay(&mut overlay, |st| st.arrivals.count == 1).await?;
        assert_eq!(
            s.arrivals.oldest_unseen.as_ref().map(|c| c.id.as_str()),
            Some(m14_id.as_str())
        );

        report_visible_current(&live, |_| vec![m14_uuid], VisibilityContinuity::Continuous).await?;
        let s = wait_overlay(&mut overlay, |st| st.arrivals.count == 0).await?;
        assert!(s.arrivals.oldest_unseen.is_none());
        assert!(s.arrivals.live_head.is_none());
        assert!(s.catch_up.available, "older catch-up must remain intact");
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn catch_up_boundary_moves_position_saves_and_arrivals_emit_zero_item_diffs() -> Result {
    let fixture = Fixture::new(10, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("ui-zero-diffs").await?;
        let mut live_cfg = config();
        live_cfg.page_size = 10;
        let live = Timeline::live(session.clone(), channel(), live_cfg).await?;
        let live_sub = live.subscribe()?;
        let all_ids = ids(&live_sub);
        report_visible_current(&live, |v| vec![v[0]], VisibilityContinuity::Continuous).await?;
        report_visible_current(
            &live,
            |v| v[8..=9].to_vec(),
            VisibilityContinuity::Discontinuous,
        )
        .await?;

        let m5_cursor = fixture.messages[4].cursor().unwrap();
        let focused = Timeline::focused(session.clone(), channel(), m5_cursor, config()).await?;
        let mut items_sub = focused.subscribe()?;
        let mut overlay_sub = focused.subscribe_reading()?;

        // 1) Boundary move
        report_visible_current(
            &live,
            |v| v[1..=2].to_vec(),
            VisibilityContinuity::Discontinuous,
        )
        .await?;
        wait_overlay(&mut overlay_sub, |st| {
            st.catch_up.start_after_id == Some(all_ids[2])
        })
        .await?;

        // 2) Seeking toggle
        let seek = focused.seek_catch_up(config())?;
        seek.cancel();
        let _ = seek.finish().await;

        // 3) Save & clear reading position
        session
            .save_reading_position(
                channel(),
                Some(ReadingPosition {
                    message_id: all_ids[4],
                    offset_units: -25,
                }),
            )
            .await?;
        session.save_reading_position(channel(), None).await?;

        // 4) Live arrival behind focused window + clear catch-up
        let foreign_author = fixture.messages[0].author_id.as_deref().unwrap().to_owned();
        let m11 = extra_message(21, &foreign_author, "2026-01-01T02:00:00.000010Z");
        fixture.server().publish_message(m11).await?;
        wait_overlay(&mut overlay_sub, |st| st.arrivals.count >= 1).await?;
        focused.clear_catch_up().await?;
        wait_overlay(&mut overlay_sub, |st| !st.catch_up.available).await?;

        // None of those operations may emit non-empty TimelineItem diffs on `Timeline::subscribe()`.
        while let Some(Some(event)) = items_sub.stream.next().now_or_never() {
            if let TimelineEvent::Update(update) = event {
                assert!(
                    update.diffs.is_empty(),
                    "unexpected {} TimelineItem diffs",
                    update.diffs.len()
                );
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn preexisting_unread_tail_is_not_counted_as_arrivals_until_new_arrival() -> Result {
    let fixture = Fixture::new(15, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("ui-preexisting").await?;
        let m10_id = fixture.messages[9].id.clone().unwrap();
        let m10_at = fixture.messages[9].created_at.clone().unwrap();
        fixture
            .server()
            .set_read_mark(
                &session.account().user().to_string(),
                CHANNEL,
                &m10_id,
                &m10_at,
            )
            .await?;
        reconnect_online(&session).await?;

        let mut cfg = config();
        cfg.page_size = 5;
        let live = Timeline::live(session.clone(), channel(), cfg.clone()).await?;
        assert_eq!(live.reading_state()?.arrivals.count, 0);

        let m5_cursor = fixture.messages[4].cursor().unwrap();
        let focused = Timeline::focused(session.clone(), channel(), m5_cursor, cfg).await?;
        let mut live_overlay = live.subscribe_reading()?;
        let mut focused_overlay = focused.subscribe_reading()?;
        assert_eq!(live_overlay.state.arrivals.count, 0);
        assert_eq!(focused_overlay.state.arrivals.count, 0);

        let foreign_author = fixture.messages[0].author_id.as_deref().unwrap().to_owned();
        let m16 = extra_message(16, &foreign_author, "2026-01-01T02:00:00.000016Z");
        let m16_id = m16.id.clone().unwrap();
        fixture.server().publish_message(m16).await?;

        let s_live = wait_overlay(&mut live_overlay, |st| st.arrivals.count == 1).await?;
        let s_focus = wait_overlay(&mut focused_overlay, |st| st.arrivals.count == 1).await?;
        assert_eq!(
            s_live
                .arrivals
                .oldest_unseen
                .as_ref()
                .map(|c| c.id.as_str()),
            Some(m16_id.as_str())
        );
        assert_eq!(
            s_focus.arrivals.live_head.as_ref().map(|c| c.id.as_str()),
            Some(m16_id.as_str())
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
