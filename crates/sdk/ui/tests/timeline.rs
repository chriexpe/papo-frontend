//! Public Timeline behavior over real loopback TCP; deterministic projection properties live in unit tests.

#[path = "timeline/cancellation.rs"]
mod cancellation;
#[path = "timeline/lifecycle.rs"]
mod lifecycle;
#[path = "timeline/outgoing.rs"]
mod outgoing;
#[path = "timeline/outgoing_faults.rs"]
mod outgoing_faults;
#[path = "timeline/search.rs"]
mod search;
#[path = "timeline/support.rs"]
mod support;

use std::sync::Arc;

use papo_sdk::{HistoryError, Message, ReadingError, Uuid};
use papo_sdk_ui::{Timeline, TimelineDirection, TimelineError};

use support::{Fixture, Result, channel, config, ids, payload, until};

#[test]
fn public_live_subscription_reflects_append_edit_delete_and_keeps_unchanged_payload_identity()
-> Result {
    let fixture = Fixture::new(12, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("timeline-live").await?;
        let timeline = Timeline::live(session, channel(), config()).await?;
        let mut subscription = timeline.subscribe()?;
        let initial = ids(&subscription);
        assert_eq!(initial.len(), 4);
        let append_id = Uuid::parse_str("90000000-0000-4000-8000-000000000001")?;
        fixture
            .server()
            .publish_message(Message {
                id: Some(append_id.to_string()),
                channel_id: Some(support::CHANNEL.into()),
                author_id: fixture.messages[0].author_id.clone(),
                content: Some(String::new()),
                created_at: Some("2026-01-01T00:00:02.123456Z".into()),
                ..Message::default()
            })
            .await?;
        until(&mut subscription, |state| {
            ids(state).last() == Some(&append_id)
        })
        .await?;
        assert_eq!(ids(&subscription).len(), 5);
        let edited_id = initial[1];
        let unchanged_id = initial[2];
        let unchanged = Arc::clone(payload(&subscription, unchanged_id).ok_or("message missing")?);
        let before_edit = Arc::clone(payload(&subscription, edited_id).ok_or("message missing")?);
        let edited_at = "2026-01-01T00:00:03.123456Z";
        fixture
            .server()
            .edit_message(&edited_id.to_string(), "", edited_at)
            .await?;
        until(&mut subscription, |state| {
            payload(state, edited_id)
                .is_some_and(|message| message.edited_at.as_deref() == Some(edited_at))
        })
        .await?;
        assert!(!Arc::ptr_eq(
            &before_edit,
            payload(&subscription, edited_id).unwrap()
        ));
        assert!(Arc::ptr_eq(
            &unchanged,
            payload(&subscription, unchanged_id).unwrap()
        ));
        assert!(
            fixture
                .server()
                .delete_message(&edited_id.to_string())
                .await?
        );
        until(&mut subscription, |state| !ids(state).contains(&edited_id)).await?;
        assert!(Arc::ptr_eq(
            &unchanged,
            payload(&subscription, unchanged_id).unwrap()
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn public_live_and_focused_windows_share_payloads_but_paginate_independently() -> Result {
    let fixture = Fixture::new(40, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("timeline-windows").await?;
        let live = Timeline::live(session.clone(), channel(), config()).await?;
        let focus = fixture.messages[18]
            .cursor()
            .ok_or("fixture cursor missing")?;
        let focused = Timeline::focused(session, channel(), focus, config()).await?;
        let live_before = ids(&live.subscribe()?);
        let focused_before = ids(&focused.subscribe()?);
        assert!(
            focused
                .paginate(TimelineDirection::Older)
                .await?
                .message_count
                > 0
        );
        assert!(
            focused
                .paginate(TimelineDirection::Newer)
                .await?
                .message_count
                > 0
        );
        let focused_after = ids(&focused.subscribe()?);
        assert!(focused_after.len() > focused_before.len());
        assert_eq!(ids(&live.subscribe()?), live_before);
        assert!(live.paginate(TimelineDirection::Older).await?.message_count > 0);
        assert_eq!(ids(&focused.subscribe()?), focused_after);
        let live_after = live.subscribe()?;
        assert!(ids(&live_after).len() > live_before.len());
        let focused_after = focused.subscribe()?;
        // Warm the gap between independently fetched neighborhoods using only the public API.
        for _ in 0..4 {
            live.paginate(TimelineDirection::Older).await?;
        }
        let live_shared = live.subscribe()?;
        let shared = ids(&focused_after)
            .into_iter()
            .find(|id| payload(&live_shared, *id).is_some())
            .ok_or("windows did not overlap")?;
        assert!(Arc::ptr_eq(
            payload(&live_shared, shared).unwrap(),
            payload(&focused_after, shared).unwrap()
        ));
        assert_eq!(ids(&focused.subscribe()?), ids(&focused_after));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn public_visible_reports_reject_old_canonical_and_window_versions_without_advancing_read_state()
-> Result {
    let fixture = Fixture::new(40, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("timeline-stale").await?;
        let timeline = Timeline::live(session.clone(), channel(), config()).await?;
        let history = session.history(channel()).await?;
        let raw = history.subscribe().await?.snapshot;
        let mut baseline = timeline.subscribe()?;
        let old_window_version = baseline.version;
        let id = ids(&baseline)[0];
        let edited_at = "2026-01-01T00:00:04.123456Z";
        fixture
            .server()
            .edit_message(&id.to_string(), "", edited_at)
            .await?;
        until(&mut baseline, |state| {
            payload(state, id)
                .is_some_and(|message| message.edited_at.as_deref() == Some(edited_at))
        })
        .await?;
        assert!(matches!(
            session
                .report_visible(channel(), raw.generation, raw.version, &[id])
                .await,
            Err(ReadingError::History(HistoryError::StaleSnapshot))
        ));
        assert!(matches!(
            timeline
                .report_visible(old_window_version, baseline.generation, &[id])
                .await,
            Err(TimelineError::StaleSnapshot)
        ));
        let before_page = timeline.subscribe()?;
        timeline.paginate(TimelineDirection::Older).await?;
        assert!(matches!(
            timeline
                .report_visible(before_page.version, before_page.generation, &[id])
                .await,
            Err(TimelineError::StaleSnapshot)
        ));
        assert!(
            session
                .reading_state(channel())
                .is_none_or(|state| state.local_point.is_none())
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[path = "timeline/people.rs"]
mod people;
