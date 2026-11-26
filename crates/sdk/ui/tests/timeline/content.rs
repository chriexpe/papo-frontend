//! Public timeline `Arc<MessageContent>` identity, sharing across windows/diffs and account isolation.

use std::sync::Arc;

use papo_sdk::{ConnectionState, Message, Session, Uuid};
use papo_sdk_ui::{
    Timeline, TimelineItem, TimelineSubscription, VisibilityContinuity,
    message_content::{Block, Inline, MessageContent},
};

use super::support::{CHANNEL, Fixture, Result, channel, config, login, online, until};

fn content_arc(subscription: &TimelineSubscription, id: Uuid) -> Option<Arc<MessageContent>> {
    subscription
        .items
        .iter()
        .find_map(|item| match item.as_ref() {
            TimelineItem::Message {
                id: current,
                content,
                ..
            } if *current == id => Some(Arc::clone(content)),
            _ => None,
        })
}

fn wire_arc(subscription: &TimelineSubscription, id: Uuid) -> Option<Arc<Message>> {
    subscription
        .items
        .iter()
        .find_map(|item| match item.as_ref() {
            TimelineItem::Message {
                id: current,
                message,
                ..
            } if *current == id => Some(Arc::clone(message)),
            _ => None,
        })
}

async fn publish_markdown_message(
    fixture: &Fixture,
    id: Uuid,
    created_at: &str,
    body: &str,
) -> Result {
    fixture
        .server()
        .publish_message(Message {
            id: Some(id.to_string()),
            channel_id: Some(CHANNEL.into()),
            author_id: fixture.messages[0].author_id.clone(),
            content: Some(body.into()),
            created_at: Some(created_at.into()),
            ..Message::default()
        })
        .await?;
    Ok(())
}

async fn reconnect_next_generation(fixture: &Fixture, session: &Session) -> Result {
    let before = session.generation();
    let mut conn = session.subscribe_connection();
    fixture.server().disconnect_gateways();
    let _ = session.reconnect();
    loop {
        let status = *conn.borrow_and_update();
        if status.state == ConnectionState::Online && status.generation > before {
            return Ok(());
        }
        conn.changed().await?;
    }
}

#[test]
fn timeline_shares_content_across_windows_subscribers_and_unrelated_diffs() -> Result {
    let fixture = Fixture::new(12, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("content-diffs").await?;
        let target_id = Uuid::parse_str("91000000-0000-4000-8000-000000000001")?;
        publish_markdown_message(
            &fixture,
            target_id,
            "2026-01-02T00:00:01.000000Z",
            "**hello** ||secret||",
        )
        .await?;

        let live = Timeline::live(session.clone(), channel(), config()).await?;
        let mut sub_a = live.subscribe()?;
        let sub_b = live.subscribe()?;
        let initial_a = content_arc(&sub_a, target_id).ok_or("target missing in sub_a")?;
        let initial_b = content_arc(&sub_b, target_id).ok_or("target missing in sub_b")?;
        assert!(Arc::ptr_eq(&initial_a, &initial_b));
        assert!(matches!(
            initial_a.blocks.as_slice(),
            [Block::Paragraph(inlines)]
                if matches!(inlines.first(), Some(Inline::Text { style, .. }) if style.bold)
        ));

        let focus_cursor = wire_arc(&sub_a, target_id)
            .and_then(|msg| msg.cursor())
            .ok_or("target cursor missing")?;
        let focused = Timeline::focused(session, channel(), focus_cursor, config()).await?;
        let mut sub_focused = focused.subscribe()?;
        let initial_focused =
            content_arc(&sub_focused, target_id).ok_or("target missing in focused")?;
        assert!(Arc::ptr_eq(&initial_a, &initial_focused));

        let before_react_wire = wire_arc(&sub_a, target_id).ok_or("wire missing")?;
        fixture
            .server()
            .set_message_reaction(
                &target_id.to_string(),
                serde_json::from_value(serde_json::json!({"unicode": "👍", "count": 1}))?,
            )
            .await?;
        until(&mut sub_a, |state| {
            wire_arc(state, target_id).is_some_and(|w| !Arc::ptr_eq(&w, &before_react_wire))
        })
        .await?;
        until(&mut sub_focused, |state| {
            wire_arc(state, target_id).is_some_and(|w| !Arc::ptr_eq(&w, &before_react_wire))
        })
        .await?;
        assert!(Arc::ptr_eq(
            &initial_a,
            &content_arc(&sub_a, target_id).ok_or("missing live react")?
        ));
        assert!(Arc::ptr_eq(
            &initial_a,
            &content_arc(&sub_focused, target_id).ok_or("missing focused react")?
        ));

        let append_id = Uuid::parse_str("91000000-0000-4000-8000-000000000002")?;
        publish_markdown_message(&fixture, append_id, "2026-01-02T00:00:02.000000Z", "later")
            .await?;
        until(&mut sub_a, |state| content_arc(state, append_id).is_some()).await?;
        assert!(Arc::ptr_eq(
            &initial_a,
            &content_arc(&sub_a, target_id).ok_or("missing after append")?
        ));

        let current = live.subscribe()?;
        live.report_visible(
            current.version,
            current.generation,
            &[target_id, append_id],
            VisibilityContinuity::Continuous,
        )
        .await?;
        assert!(Arc::ptr_eq(
            &initial_a,
            &content_arc(&live.subscribe()?, target_id).ok_or("missing after read")?
        ));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn timeline_preserves_content_across_reconnect_and_replaces_on_edit() -> Result {
    let fixture = Fixture::new(12, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("content-edit").await?;
        let target_id = Uuid::parse_str("91000000-0000-4000-8000-000000000003")?;
        publish_markdown_message(
            &fixture,
            target_id,
            "2026-01-02T00:00:01.000000Z",
            "**before**",
        )
        .await?;

        let live = Timeline::live(session.clone(), channel(), config()).await?;
        let mut sub_live = live.subscribe()?;
        let before = content_arc(&sub_live, target_id).ok_or("missing initial")?;
        let cursor = wire_arc(&sub_live, target_id)
            .and_then(|m| m.cursor())
            .ok_or("missing cursor")?;
        let focused = Timeline::focused(session.clone(), channel(), cursor, config()).await?;
        let mut sub_focused = focused.subscribe()?;

        reconnect_next_generation(&fixture, &session).await?;
        let reopened = Timeline::live(session, channel(), config()).await?;
        let reopened_content =
            content_arc(&reopened.subscribe()?, target_id).ok_or("missing after reconnect")?;
        assert!(Arc::ptr_eq(&before, &reopened_content));

        let edited_at = "2026-01-02T00:00:05.000000Z";
        fixture
            .server()
            .edit_message(&target_id.to_string(), "_edited_ body", edited_at)
            .await?;
        until(&mut sub_live, |state| {
            wire_arc(state, target_id).is_some_and(|w| w.edited_at.as_deref() == Some(edited_at))
        })
        .await?;
        until(&mut sub_focused, |state| {
            wire_arc(state, target_id).is_some_and(|w| w.edited_at.as_deref() == Some(edited_at))
        })
        .await?;
        let edited_live = content_arc(&sub_live, target_id).ok_or("missing live edit")?;
        let edited_focused = content_arc(&sub_focused, target_id).ok_or("missing focused edit")?;
        assert!(!Arc::ptr_eq(&before, &edited_live));
        assert!(Arc::ptr_eq(&edited_live, &edited_focused));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn timeline_content_cache_isolates_accounts_on_same_channel() -> Result {
    let fixture = Fixture::new(6, vec![])?;
    fixture.run(async {
        let alice = fixture.signed_in("content-alice").await?;
        let handle = fixture.client().handle();
        let auth = handle.auth(alice.account().server())?;
        let password: String = std::iter::once('A')
            .chain((0_u8..16).map(|index| char::from(b'a' + index)))
            .chain(std::iter::once('!'))
            .collect();
        auth.register(papo_sdk::RegisterRequest {
            username: "content-bob".into(),
            password,
        })
        .await?;
        let bob = login(&auth, "content-bob").await?;
        online(&bob).await?;

        let target_id: Uuid = fixture.messages[5]
            .id
            .as_deref()
            .ok_or("missing seeded id")?
            .parse()?;
        let alice_live = Timeline::live(alice, channel(), config()).await?;
        let bob_live = Timeline::live(bob, channel(), config()).await?;

        let alice_content =
            content_arc(&alice_live.subscribe()?, target_id).ok_or("missing alice content")?;
        let bob_content =
            content_arc(&bob_live.subscribe()?, target_id).ok_or("missing bob content")?;
        assert!(!Arc::ptr_eq(&alice_content, &bob_content));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_server_emoji_resolves_in_the_timeline_and_its_reply_line() -> Result {
    let fixture = Fixture::new(4, vec![])?;
    fixture.run(async {
        let gato = Uuid::parse_str("e0000000-0000-4000-8000-000000000001")?;
        fixture
            .server()
            .set_emojis(vec![papo_test_server::ServerEmoji {
                id: gato.to_string(),
                name: "gato".into(),
                format: "png".into(),
                created_at: "2026-01-01T00:00:00Z".into(),
                image: vec![1, 2, 3],
            }])
            .await;
        let session = fixture.signed_in("content-emoji").await?;
        let original = Uuid::parse_str("91000000-0000-4000-8000-0000000000e1")?;
        let reply = Uuid::parse_str("91000000-0000-4000-8000-0000000000e2")?;
        publish_markdown_message(&fixture, original, "2026-01-02T00:00:01.000000Z", ":gato:")
            .await?;
        fixture
            .server()
            .publish_message(Message {
                id: Some(reply.to_string()),
                channel_id: Some(CHANNEL.into()),
                author_id: fixture.messages[0].author_id.clone(),
                content: Some("same".into()),
                reply_to: Some(original.to_string()),
                created_at: Some("2026-01-02T00:00:02.000000Z".into()),
                ..Message::default()
            })
            .await?;
        let resolved = |content: &MessageContent| {
            matches!(
                content.blocks.as_slice(),
                [Block::Paragraph(inlines)]
                    if matches!(inlines.as_slice(), [Inline::CustomEmoji { id, .. }] if *id == gato)
            )
        };
        let live = Timeline::live(session, channel(), config()).await?;
        let mut subscription = live.subscribe()?;
        // The catalog lists on connect; the window resolves once it lands.
        until(&mut subscription, |state| {
            content_arc(state, original).is_some_and(|content| resolved(&content))
        })
        .await?;
        let reply_original = subscription
            .items
            .iter()
            .find_map(|item| match item.as_ref() {
                TimelineItem::Message {
                    id,
                    reply: Some(papo_sdk_ui::ReplyOriginal::Held { content, .. }),
                    ..
                } if *id == reply => Some(Arc::clone(content)),
                _ => None,
            });
        assert!(reply_original.is_some_and(|content| resolved(&content)));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
