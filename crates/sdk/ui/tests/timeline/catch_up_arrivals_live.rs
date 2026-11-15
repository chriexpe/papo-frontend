//! Regression spec: a focused window opened before live-edge hydration counts the first WS arrival.

use super::catch_up::{extra_message, wait_overlay};
use super::support::{Fixture, Result, channel, config};
use papo_sdk_ui::{Timeline, TimelineDirection};

#[test]
fn focused_window_opened_before_live_edge_counts_first_ws_arrival_and_isolates_rest_fill() -> Result
{
    let fixture = Fixture::new(10, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("ui-ws-unhydrated").await?;
        let mut focus_cfg = config();
        focus_cfg.page_size = 2;
        let m3_cursor = fixture.messages[2].cursor().unwrap();
        let focused =
            Timeline::focused(session.clone(), channel(), m3_cursor, focus_cfg.clone()).await?;
        let mut overlay = focused.subscribe_reading()?;
        assert_eq!(overlay.state.arrivals.count, 0);

        let foreign_author = fixture.messages[0].author_id.as_deref().unwrap().to_owned();
        let m11 = extra_message(41, &foreign_author, "2026-01-01T02:00:00.000011Z");
        let m11_id = m11.id.clone().unwrap();
        fixture.server().publish_message(m11).await?;

        let s = wait_overlay(&mut overlay, |st| st.arrivals.count == 1).await?;
        assert_eq!(
            s.arrivals.oldest_unseen.as_ref().map(|c| c.id.as_str()),
            Some(m11_id.as_str())
        );
        assert_eq!(
            s.arrivals.live_head.as_ref().map(|c| c.id.as_str()),
            Some(m11_id.as_str())
        );

        // Subsequent REST hydration of the pre-existing [m9..=m10] tail must neither inflate
        // focused arrivals nor swallow m11, while a newly opened live window starts at 0.
        let mut live_cfg = config();
        live_cfg.page_size = 3;
        let live = Timeline::live(session.clone(), channel(), live_cfg).await?;
        assert_eq!(live.reading_state()?.arrivals.count, 0);
        let focus_after_rest = focused.reading_state()?;
        assert_eq!(focus_after_rest.arrivals.count, 1);
        assert_eq!(
            focus_after_rest
                .arrivals
                .oldest_unseen
                .as_ref()
                .map(|c| c.id.as_str()),
            Some(m11_id.as_str())
        );

        let m12 = extra_message(42, &foreign_author, "2026-01-01T02:00:00.000012Z");
        let m12_id = m12.id.clone().unwrap();
        fixture.server().publish_message(m12).await?;
        let s2 = wait_overlay(&mut overlay, |st| st.arrivals.count == 2).await?;
        assert_eq!(
            s2.arrivals.oldest_unseen.as_ref().map(|c| c.id.as_str()),
            Some(m11_id.as_str())
        );
        assert_eq!(
            s2.arrivals.live_head.as_ref().map(|c| c.id.as_str()),
            Some(m12_id.as_str())
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn focused_window_with_ws_arrival_ignores_intermediate_non_terminal_rest_fill() -> Result {
    let fixture = Fixture::new(10, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("ui-partial-rest").await?;
        let mut focus_cfg = config();
        focus_cfg.page_size = 2;
        let m3_cursor = fixture.messages[2].cursor().unwrap();
        let focused = Timeline::focused(session.clone(), channel(), m3_cursor, focus_cfg).await?;
        let mut overlay = focused.subscribe_reading()?;
        assert_eq!(overlay.state.arrivals.count, 0);

        let foreign_author = fixture.messages[0].author_id.as_deref().unwrap().to_owned();
        let m11 = extra_message(43, &foreign_author, "2026-01-01T02:00:00.000011Z");
        let m11_id = m11.id.clone().unwrap();
        fixture.server().publish_message(m11).await?;
        let s = wait_overlay(&mut overlay, |st| st.arrivals.count == 1).await?;
        assert_eq!(
            s.arrivals.oldest_unseen.as_ref().map(|c| c.id.as_str()),
            Some(m11_id.as_str())
        );

        // Partial newer page loads pre-existing [m5, m6] with `has_more = true` (`!server_live_complete`).
        // Those pre-existing rows must not inflate arrivals or move `oldest_unseen` behind m11.
        let page = focused.paginate(TimelineDirection::Newer).await?;
        assert!(page.has_more);
        let state = focused.reading_state()?;
        assert_eq!(state.arrivals.count, 1);
        assert_eq!(
            state.arrivals.oldest_unseen.as_ref().map(|c| c.id.as_str()),
            Some(m11_id.as_str())
        );
        assert_eq!(
            state.arrivals.live_head.as_ref().map(|c| c.id.as_str()),
            Some(m11_id.as_str())
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
