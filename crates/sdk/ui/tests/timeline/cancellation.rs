//! Public pagination cancellation clears view-local loading even while the HTTP response is held.

use papo_sdk_ui::{Timeline, TimelineDirection, TimelineItem};

use super::support::{Fixture, Result, channel, config, ids, until};

#[test]
fn cancelled_public_pagination_clears_loading_and_does_not_extend_its_window() -> Result {
    let fixture = Fixture::new(80, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("timeline-cancel").await?;
        let timeline = Timeline::live(session.clone(), channel(), config()).await?;
        let before = ids(&timeline.subscribe()?);
        let mut held = fixture.server().hold_next_rest("GET", &format!("/channels/{}/messages", super::support::CHANNEL)).await?;
        let mut pagination = Box::pin(timeline.paginate(TimelineDirection::Older));
        tokio::select! {
            started = held.wait_started() => started?,
            _ = &mut pagination => return Err("pagination completed before held request started".into()),
        }
        let mut subscription = timeline.subscribe()?;
        assert!(subscription.items.iter().any(|item| matches!(item.as_ref(), TimelineItem::Loading(TimelineDirection::Older))));
        drop(pagination);
        until(&mut subscription, |state| !state.items.iter().any(|item| matches!(item.as_ref(), TimelineItem::Loading(_)))).await?;
        assert_eq!(ids(&subscription), before);
        assert!(session.reading_state(channel()).is_none_or(|state| state.local_point.is_none()));
        held.release();
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
