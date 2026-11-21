//! Windows opened at a saved reading position's anchor (`live_at`, `focused_at`), over real
//! loopback TCP. Echo replacement and focused-window paging are covered in `outgoing.rs` and
//! `timeline.rs`; here only what opening by ID adds.

use super::support::{CHANNEL, Fixture, Result, channel, config, ids, online, until};
use papo_sdk::{
    Message, OperationKind, OutgoingMessage, ReadingPosition, Session, UnixMicros, Uuid,
};
use papo_sdk_ui::{Timeline, TimelineDirection, TimelineError, TimelineItem, TimelineSubscription};

fn id(message: &Message) -> Result<Uuid> {
    Ok(Uuid::parse_str(
        message.id.as_deref().ok_or("fixture id missing")?,
    )?)
}

fn id_range(messages: &[Message]) -> Result<Vec<Uuid>> {
    messages.iter().map(id).collect()
}

/// Caches the whole channel, as reading back through it does.
async fn cache_everything(session: &Session) -> Result {
    let reader = Timeline::live(session.clone(), channel(), config()).await?;
    while reader.paginate(TimelineDirection::Older).await?.has_more {}
    Ok(())
}

fn message_path() -> String {
    format!("/channels/{}/messages", channel())
}

fn echoes(state: &TimelineSubscription, local: Uuid) -> usize {
    state
        .items
        .iter()
        .filter(|item| matches!(item.as_ref(), TimelineItem::LocalEcho { id, .. } if *id == local))
        .count()
}

#[test]
fn live_at_reaches_back_to_a_cached_anchor_and_still_follows_new_messages() -> Result {
    let fixture = Fixture::new(40, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("timeline-live-at").await?;
        cache_everything(&session).await?;

        // Messages 7 to 13 share one timestamp: the anchor sits inside the tie, and the page
        // older than it crosses the tie's edge.
        let anchor = id(&fixture.messages[10])?;
        let timeline = Timeline::live_at(session.clone(), channel(), anchor, config()).await?;
        let mut subscription = timeline.subscribe()?;
        // One page (four) older than the anchor, through the newest, nothing skipped.
        let expected = id_range(&fixture.messages[6..])?;
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

        // A local echo joins once, as in a window opened live.
        let local = Uuid::from_u128(0xd000_0000_0000_4000_8000_0000_0000_0007);
        session
            .outgoing()
            .enqueue(
                channel(),
                local,
                OperationKind::Message(OutgoingMessage {
                    content: "echo".into(),
                    ..Default::default()
                }),
                UnixMicros(0),
            )
            .await?;
        until(&mut subscription, |state| echoes(state, local) > 0).await?;
        assert_eq!(echoes(&subscription, local), 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_cached_anchor_past_the_live_reach_opens_in_a_bounded_neighborhood() -> Result {
    let fixture = Fixture::new(120, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("live-at-reach").await?;
        cache_everything(&session).await?;

        // Twenty pages of four: an anchor with 79 newer cached messages is the farthest reach.
        let farthest = id(&fixture.messages[40])?;
        let timeline = Timeline::live_at(session.clone(), channel(), farthest, config()).await?;
        assert_eq!(
            ids(&timeline.subscribe()?),
            id_range(&fixture.messages[36..])?
        );

        // One message further back is still cached and restorable, but not live.
        let beyond = id(&fixture.messages[39])?;
        let opened = Timeline::live_at(session.clone(), channel(), beyond, config()).await;
        assert!(matches!(opened, Err(TimelineError::AnchorTooFar)));
        let focused = Timeline::focused_at(session, channel(), beyond, config()).await?;
        let neighborhood = ids(&focused.subscribe()?);
        assert!(neighborhood.contains(&beyond));
        // A page either side of the anchor, never the 80 messages up to the newest.
        assert!(neighborhood.len() <= 2 * usize::from(config().page_size));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn an_unknown_anchor_is_missing_and_a_deleted_one_says_so() -> Result {
    let fixture = Fixture::new(40, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("live-at-missing").await?;
        // Only the newest page is cached.
        let reader = Timeline::live(session.clone(), channel(), config()).await?;
        let mut newest = reader.subscribe()?;

        // There is no GET-by-ID route: an uncached anchor cannot be found, however close.
        let uncached = id(&fixture.messages[5])?;
        let unknown = Uuid::from_u128(7);
        for anchor in [uncached, unknown] {
            let live = Timeline::live_at(session.clone(), channel(), anchor, config()).await;
            assert!(matches!(live, Err(TimelineError::FocusMissing)));
            let focused = Timeline::focused_at(session.clone(), channel(), anchor, config()).await;
            assert!(matches!(focused, Err(TimelineError::FocusMissing)));
        }

        let deleted = id(&fixture.messages[38])?;
        assert!(
            fixture
                .server()
                .delete_message(&deleted.to_string())
                .await?
        );
        until(&mut newest, |state| !ids(state).contains(&deleted)).await?;
        let live = Timeline::live_at(session.clone(), channel(), deleted, config()).await;
        assert!(matches!(live, Err(TimelineError::AnchorDeleted)));
        let focused = Timeline::focused_at(session, channel(), deleted, config()).await;
        assert!(matches!(focused, Err(TimelineError::AnchorDeleted)));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn an_anchor_deleted_while_its_window_opens_is_reported_deleted() -> Result {
    let fixture = Fixture::new(40, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("live-at-race").await?;
        // Opening refetches the newest page, then the page older than message 36: both are
        // held, so the deletion lands after the anchor was located.
        let reader = Timeline::live(session.clone(), channel(), config()).await?;
        let mut newest_page = fixture
            .server()
            .hold_next_rest("GET", &message_path())
            .await?;
        let mut older_page = fixture
            .server()
            .hold_next_rest("GET", &message_path())
            .await?;
        let anchor = id(&fixture.messages[36])?;
        let opening = tokio::spawn(Timeline::live_at(
            session.clone(),
            channel(),
            anchor,
            config(),
        ));
        newest_page.wait_started().await?;
        newest_page.release();
        // The anchor was located; the page older than it is on its way.
        older_page.wait_started().await?;
        assert!(fixture.server().delete_message(&anchor.to_string()).await?);
        let mut newest = reader.subscribe()?;
        until(&mut newest, |state| !ids(state).contains(&anchor)).await?;
        older_page.release();
        assert!(matches!(opening.await?, Err(TimelineError::AnchorDeleted)));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_gap_between_the_anchor_and_the_newest_stays_a_gap() -> Result {
    let fixture = Fixture::new(40, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("live-at-gap").await?;
        // Two cached runs: around message 10 (read through a reply or a search) and the
        // newest page; nothing between them is known.
        let around = fixture.messages[10]
            .cursor()
            .ok_or("fixture cursor missing")?;
        let _reply = Timeline::focused(session.clone(), channel(), around, config()).await?;
        let anchor = id(&fixture.messages[10])?;
        let timeline = Timeline::live_at(session, channel(), anchor, config()).await?;
        let items = timeline.subscribe()?.items;
        let position = |wanted: Uuid| {
            items.iter().position(
                |item| matches!(item.as_ref(), TimelineItem::Message { id, .. } if *id == wanted),
            )
        };
        let at_anchor = position(anchor).ok_or("anchor missing")?;
        let at_newest = position(id(&fixture.messages[39])?).ok_or("newest missing")?;
        assert!(
            items
                .iter()
                .take(at_newest)
                .skip(at_anchor)
                .any(|item| matches!(item.as_ref(), TimelineItem::Gap(_))),
            "a cached anchor proves no continuity up to the newest"
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_saved_reading_position_reopens_after_a_restart_and_is_left_unchanged() -> Result {
    let mut fixture = Fixture::new(40, vec![])?;
    let anchor = id(&fixture.messages[20])?;
    let saved = ReadingPosition {
        message_id: anchor,
        offset_units: -12,
    };
    let account = fixture.run(async {
        let session = fixture.signed_in("live-at-restart").await?;
        cache_everything(&session).await?;
        session
            .save_reading_position(channel(), Some(saved))
            .await?;
        Ok::<_, Box<dyn std::error::Error>>(session.account())
    })??;
    fixture.reopen()?;
    fixture.run(async {
        let session = fixture.client().handle().session(account)?;
        online(&session).await?;
        let mut positions = session.subscribe_reading_positions();
        while !positions
            .borrow_and_update()
            .channels
            .contains_key(&channel())
        {
            positions.changed().await?;
        }
        let restored = session.reading_position(channel()).ok_or("position lost")?;
        assert_eq!(restored, saved);
        let timeline =
            Timeline::live_at(session.clone(), channel(), restored.message_id, config()).await?;
        assert!(ids(&timeline.subscribe()?).contains(&anchor));
        // Opening, like fetching, never touches the saved position.
        assert_eq!(session.reading_position(channel()), Some(saved));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
