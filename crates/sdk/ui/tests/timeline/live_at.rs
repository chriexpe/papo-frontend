//! Live windows opened at a saved reading position's anchor, over real loopback TCP.

use super::support::{CHANNEL, Fixture, Result, channel, config, ids, until};
use papo_sdk::{Message, Uuid};
use papo_sdk_ui::{Timeline, TimelineDirection, TimelineError};

fn id(message: &Message) -> Result<Uuid> {
    Ok(Uuid::parse_str(
        message.id.as_deref().ok_or("fixture id missing")?,
    )?)
}

#[test]
fn live_at_reaches_back_to_a_cached_anchor_and_still_follows_new_messages() -> Result {
    let fixture = Fixture::new(40, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("timeline-live-at").await?;
        // Cache all but the oldest four messages, as reading back through a channel does.
        let reader = Timeline::live(session.clone(), channel(), config()).await?;
        for _ in 0..8 {
            reader.paginate(TimelineDirection::Older).await?;
        }
        drop(reader);

        // Messages 7 to 13 share one timestamp: the anchor sits inside the tie, and the page
        // older than it crosses the tie's edge.
        let anchor = id(&fixture.messages[10])?;
        let timeline = Timeline::live_at(session, channel(), anchor, config()).await?;
        let mut subscription = timeline.subscribe()?;
        // One page (four) older than the anchor, through the newest, nothing skipped.
        let expected = fixture.messages[6..]
            .iter()
            .map(id)
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(ids(&subscription), expected);

        let arrival = Uuid::parse_str("90000000-0000-4000-8000-000000000002")?;
        fixture
            .server()
            .publish_message(Message {
                id: Some(arrival.to_string()),
                channel_id: Some(CHANNEL.into()),
                author_id: fixture.messages[0].author_id.clone(),
                content: Some(String::new()),
                created_at: Some("2026-01-01T00:00:02.123456Z".into()),
                ..Message::default()
            })
            .await?;
        until(&mut subscription, |state| {
            ids(state).last() == Some(&arrival)
        })
        .await?;
        assert_eq!(ids(&subscription).first(), expected.first());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn live_at_needs_an_anchor_that_is_cached_and_not_deleted() -> Result {
    let fixture = Fixture::new(40, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("live-at-missing").await?;
        // Only the newest page is cached.
        let reader = Timeline::live(session.clone(), channel(), config()).await?;
        let mut newest = reader.subscribe()?;

        let uncached = id(&fixture.messages[5])?;
        let unknown = Uuid::from_u128(7);
        for anchor in [uncached, unknown] {
            let opened = Timeline::live_at(session.clone(), channel(), anchor, config()).await;
            assert!(matches!(opened, Err(TimelineError::FocusMissing)));
        }

        let deleted = id(&fixture.messages[38])?;
        assert!(
            fixture
                .server()
                .delete_message(&deleted.to_string())
                .await?
        );
        until(&mut newest, |state| !ids(state).contains(&deleted)).await?;
        let opened = Timeline::live_at(session, channel(), deleted, config()).await;
        assert!(matches!(opened, Err(TimelineError::FocusMissing)));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn live_at_reaches_from_inside_the_newest_page_but_not_past_twenty_pages() -> Result {
    let fixture = Fixture::new(120, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("live-at-reach").await?;
        let reader = Timeline::live(session.clone(), channel(), config()).await?;
        while reader.paginate(TimelineDirection::Older).await?.has_more {}
        drop(reader);

        // Inside the newest page: one page older than the anchor joins it.
        let near = id(&fixture.messages[118])?;
        let timeline = Timeline::live_at(session.clone(), channel(), near, config()).await?;
        let expected = fixture.messages[114..]
            .iter()
            .map(id)
            .collect::<Result<Vec<_>>>()?;
        assert_eq!(ids(&timeline.subscribe()?), expected);

        // Twenty pages of four reach 80 messages back from the newest; this one is 110 back.
        let far = id(&fixture.messages[9])?;
        let opened = Timeline::live_at(session, channel(), far, config()).await;
        assert!(matches!(opened, Err(TimelineError::FocusMissing)));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
