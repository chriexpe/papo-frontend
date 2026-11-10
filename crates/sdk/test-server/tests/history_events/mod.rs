//! Owns live/scoped/disconnected history proofs on loopback TCP; leaves gateway timing to duplex unit tests.

use futures_util::StreamExt;
use papo_api::{
    gateway::{Event, HeartbeatAckEvent, KnownEvent, MessageEditEvent},
    wire::{MessageListResponse, MessageReactionSummary, PinnedMessageList},
};
use papo_test_server::{Config, Error, TestServer};
use serde_json::Value;

use super::{
    AT, CHANNEL, seed,
    test_client::{Account, next, resource_event},
};

#[tokio::test]
async fn mutations_emit_only_pinned_backend_payloads_and_only_to_readable_accounts()
-> Result<(), Error> {
    let server = TestServer::start(Config {
        history: vec![seed(2)],
        ..Config::default()
    })
    .await?;
    let first = Account::register(&server, "reader").await?;
    let second = Account::register(&server, "denied").await?;
    server
        .set_channel_readable(&second.id, CHANNEL, false)
        .await?;
    let mut first_socket = first.socket(&server).await?;
    let mut second_socket = second.socket(&server).await?;
    assert_eq!(next(&mut first_socket).await?.kind(), "presence_sync");
    assert_eq!(next(&mut second_socket).await?.kind(), "presence_sync");
    let message = seed(3).messages()?.pop().ok_or(Error::Configuration)?;
    let id = message.id.clone().ok_or(Error::Configuration)?;
    server.publish_message(message).await?;
    let Event::Known(KnownEvent::Message(event)) = resource_event(&mut first_socket).await? else {
        return Err(Error::Connection);
    };
    assert_eq!(event.id, id);
    assert_eq!(event.created_at, AT);
    server.set_message_embeds(&id, Vec::new()).await?;
    assert_eq!(
        resource_event(&mut first_socket).await?.kind(),
        "message_embeds_update"
    );
    server
        .set_message_reaction(
            &id,
            MessageReactionSummary {
                unicode: Some(
                    char::from_u32(0x1f44d)
                        .ok_or(Error::Configuration)?
                        .to_string(),
                ),
                count: Some(2),
                ..MessageReactionSummary::default()
            },
        )
        .await?;
    assert_reaction_payload(resource_event(&mut first_socket).await?, &id)?;
    server.set_message_pin(&id, Some(AT)).await?;
    let Event::Known(KnownEvent::MessagePin(event)) = resource_event(&mut first_socket).await?
    else {
        return Err(Error::Connection);
    };
    let payload = serde_json::to_value(event).map_err(|_| Error::Configuration)?;
    assert_eq!(payload.as_object().map(serde_json::Map::len), Some(2));
    assert_eq!(payload["is_pinned"], true);
    server
        .edit_message(&id, "", "2026-01-01T00:00:00.123457Z")
        .await?;
    assert_eq!(
        resource_event(&mut first_socket).await?.kind(),
        "message_edit"
    );
    assert_eq!(
        resource_event(&mut first_socket).await?.kind(),
        "message_embeds_update"
    );
    server.set_message_pin(&id, None).await?;
    assert_eq!(
        resource_event(&mut first_socket).await?.kind(),
        "message_pin"
    );
    assert!(server.delete_message(&id).await?);
    assert_eq!(
        resource_event(&mut first_socket).await?.kind(),
        "message_delete"
    );
    assert!(!server.delete_message(&id).await?);
    server
        .emit_event(KnownEvent::HeartbeatAck(Box::new(HeartbeatAckEvent {})))
        .await?;
    assert_eq!(
        resource_event(&mut first_socket).await?.kind(),
        "heartbeat_ack"
    );
    assert_eq!(
        resource_event(&mut second_socket).await?.kind(),
        "heartbeat_ack"
    );
    let pins: PinnedMessageList = first.get(&format!("/channels/{CHANNEL}/pinned")).await?;
    assert!(pins.pinned.ok_or(Error::Connection)?.is_empty());
    first_socket
        .close(None)
        .await
        .map_err(|_| Error::Connection)?;
    second_socket
        .close(None)
        .await
        .map_err(|_| Error::Connection)?;
    server.shutdown().await
}

#[tokio::test]
async fn disconnected_edits_deletes_embeds_reactions_and_pins_are_rest_authority_not_gateway_replay()
-> Result<(), Error> {
    let server = TestServer::start(Config {
        history: vec![seed(4)],
        ..Config::default()
    })
    .await?;
    let account = Account::register(&server, "offline").await?;
    let mut socket = account.socket(&server).await?;
    assert_eq!(next(&mut socket).await?.kind(), "presence_sync");
    server.set_gateway_accepting(false).await;
    server.disconnect_gateways();
    let closed = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .map_err(|_| Error::Connection)?;
    assert!(closed.is_none_or(|frame| frame.is_err() || frame.is_ok_and(|frame| frame.is_close())));
    let messages = seed(5).messages()?;
    let id = |index: usize| messages[index].id.as_deref().ok_or(Error::Configuration);
    server
        .edit_message(id(0)?, "", "2026-01-01T00:00:01.123456Z")
        .await?;
    assert!(server.delete_message(id(1)?).await?);
    server.set_message_embeds(id(2)?, Vec::new()).await?;
    server
        .set_message_reaction(
            id(2)?,
            MessageReactionSummary {
                unicode: Some(
                    char::from_u32(0x1f44d)
                        .ok_or(Error::Configuration)?
                        .to_string(),
                ),
                count: Some(3),
                ..MessageReactionSummary::default()
            },
        )
        .await?;
    server
        .set_message_pin(id(3)?, Some("2026-01-01T00:00:02.123456Z"))
        .await?;
    server.publish_message(messages[4].clone()).await?;
    server.set_gateway_accepting(true).await;
    let mut socket = account.socket(&server).await?;
    assert_eq!(next(&mut socket).await?.kind(), "presence_sync");
    server
        .emit_event(KnownEvent::HeartbeatAck(Box::new(HeartbeatAckEvent {})))
        .await?;
    assert_eq!(resource_event(&mut socket).await?.kind(), "heartbeat_ack");
    let body: MessageListResponse = account
        .get(&format!("/channels/{CHANNEL}/messages?order=asc"))
        .await?;
    let body = body.messages.ok_or(Error::Connection)?;
    assert_eq!(body.len(), 4);
    assert_eq!(
        body[0].edited_at.as_deref(),
        Some("2026-01-01T00:00:01.123456Z")
    );
    assert!(
        !body
            .iter()
            .any(|message| message.id.as_deref() == Some(id(1).unwrap()))
    );
    assert_eq!(
        body[1].reactions.as_ref().ok_or(Error::Connection)?[0].count,
        Some(3)
    );
    let pins: PinnedMessageList = account.get(&format!("/channels/{CHANNEL}/pinned")).await?;
    assert_eq!(
        pins.pinned.ok_or(Error::Connection)?[0].id.as_deref(),
        Some(id(3)?)
    );
    let raw: Value = account
        .get(&format!("/channels/{CHANNEL}/messages?order=asc"))
        .await?;
    assert!(raw["messages"][0].get("deleted").is_none());
    socket.close(None).await.map_err(|_| Error::Connection)?;
    server.shutdown().await
}

fn assert_reaction_payload(event: Event, id: &str) -> Result<(), Error> {
    let Event::Known(KnownEvent::ReactUpdate(event)) = event else {
        return Err(Error::Connection);
    };
    let payload = serde_json::to_value(event).map_err(|_| Error::Configuration)?;
    assert_eq!(payload["message_id"], id);
    assert_eq!(payload["count"], 2);
    assert!(payload.get("channel_id").is_none());
    assert!(payload.get("user_id").is_none());
    Ok(())
}

#[tokio::test]
async fn stale_duplicate_event_scripts_do_not_mutate_authoritative_rest_state() -> Result<(), Error>
{
    let server = TestServer::start(Config {
        history: vec![seed(1)],
        ..Config::default()
    })
    .await?;
    let first = Account::register(&server, "script").await?;
    let id = seed(1)
        .messages()?
        .remove(0)
        .id
        .ok_or(Error::Configuration)?;
    let mut first_socket = first.socket(&server).await?;
    assert_eq!(next(&mut first_socket).await?.kind(), "presence_sync");
    let stale = KnownEvent::MessageEdit(Box::new(MessageEditEvent {
        id: id.clone(),
        channel_id: CHANNEL.into(),
        content: String::new(),
        edited_at: AT.into(),
    }));
    for _ in 0..2 {
        server.emit_user_event(&first.id, stale.clone()).await?;
        assert_eq!(
            resource_event(&mut first_socket).await?.kind(),
            "message_edit"
        );
    }
    let response: MessageListResponse = first.get(&format!("/channels/{CHANNEL}/messages")).await?;
    assert!(
        response.messages.ok_or(Error::Connection)?[0]
            .edited_at
            .is_none()
    );
    first_socket
        .close(None)
        .await
        .map_err(|_| Error::Connection)?;
    server.shutdown().await
}

#[tokio::test]
async fn nonempty_embed_metadata_replaces_and_clears_in_both_event_and_rest_payloads()
-> Result<(), Error> {
    let server = TestServer::start(Config {
        history: vec![seed(1)],
        ..Config::default()
    })
    .await?;
    let account = Account::register(&server, "embeds").await?;
    let id = seed(1)
        .messages()?
        .remove(0)
        .id
        .ok_or(Error::Configuration)?;
    let mut socket = account.socket(&server).await?;
    assert_eq!(next(&mut socket).await?.kind(), "presence_sync");
    let embed = papo_api::wire::Embed {
        id: Some("90000000-0000-4000-8000-000000000001".into()),
        source_type: Some("custom".into()),
        fetch_method: Some("manual".into()),
        created_at: Some(AT.into()),
        ..papo_api::wire::Embed::default()
    };
    server.set_message_embeds(&id, vec![embed.clone()]).await?;
    let Event::Known(KnownEvent::MessageEmbedsUpdate(event)) = resource_event(&mut socket).await?
    else {
        return Err(Error::Connection);
    };
    assert_eq!(event.embeds.len(), 1);
    assert_eq!(event.embeds[0].id, embed.id);
    let response: MessageListResponse = account
        .get(&format!("/channels/{CHANNEL}/messages"))
        .await?;
    assert_eq!(
        response.messages.ok_or(Error::Connection)?[0]
            .embeds
            .as_ref()
            .ok_or(Error::Connection)?[0]
            .id,
        embed.id
    );
    server.set_message_embeds(&id, Vec::new()).await?;
    let Event::Known(KnownEvent::MessageEmbedsUpdate(event)) = resource_event(&mut socket).await?
    else {
        return Err(Error::Connection);
    };
    assert!(event.embeds.is_empty());
    socket.close(None).await.map_err(|_| Error::Connection)?;
    server.shutdown().await
}

#[tokio::test]
async fn publishing_with_existing_embeds_emits_separate_update_and_canonical_uuid()
-> Result<(), Error> {
    let server = TestServer::start(Config {
        history: vec![seed(0)],
        ..Config::default()
    })
    .await?;
    let account = Account::register(&server, "publish").await?;
    let mut socket = account.socket(&server).await?;
    assert_eq!(next(&mut socket).await?.kind(), "presence_sync");
    let mut message = seed(1).messages()?.remove(0);
    message.id = Some("A0000000-0000-4000-8000-000000000001".into());
    message.embeds = Some(vec![papo_api::wire::Embed {
        id: Some("90000000-0000-4000-8000-000000000001".into()),
        source_type: Some("custom".into()),
        fetch_method: Some("manual".into()),
        created_at: Some(AT.into()),
        ..papo_api::wire::Embed::default()
    }]);
    server.publish_message(message).await?;
    let Event::Known(KnownEvent::Message(message)) = resource_event(&mut socket).await? else {
        return Err(Error::Connection);
    };
    assert_eq!(message.id, "a0000000-0000-4000-8000-000000000001");
    let Event::Known(KnownEvent::MessageEmbedsUpdate(event)) = resource_event(&mut socket).await?
    else {
        return Err(Error::Connection);
    };
    assert_eq!(event.message_id, message.id);
    assert_eq!(event.embeds.len(), 1);
    socket.close(None).await.map_err(|_| Error::Connection)?;
    server.shutdown().await
}
