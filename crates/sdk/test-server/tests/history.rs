//! Owns history paging and scoped REST proofs over loopback TCP; leaves pure ordering and SDK storage outside.

use papo_api::{
    HistoryOrder, HistoryQuery, RestClient, Uuid,
    wire::{
        LoginRequest, MessageListResponse, MessageReactionSummary, MessageUserReaction,
        PinnedMessageList,
    },
};
use papo_test_server::{Config, Error, HistorySeed, TestServer};
use serde_json::Value;

mod history_events;
mod test_client;
use test_client::Account;

const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
const AT: &str = "2026-01-01T00:00:00.123456Z";

fn seed(count: usize) -> HistorySeed {
    HistorySeed {
        channel_id: CHANNEL.into(),
        author_id: "20000000-0000-4000-8000-000000000001".into(),
        first_message_id: "70000000-0000-4000-8000-000000000001".into(),
        count,
        first_created_at: AT.into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 9,
    }
}

#[tokio::test]
async fn public_rest_client_pages_ten_thousand_messages_both_directions_with_exact_cursors()
-> Result<(), Error> {
    let recipe = seed(10_000);
    let expected: Vec<_> = recipe
        .messages()?
        .into_iter()
        .map(|message| message.id.unwrap())
        .collect();
    let server = TestServer::start(Config {
        history: vec![recipe],
        ..Config::default()
    })
    .await?;
    let account = Account::register(&server, "pager").await?;
    let client = RestClient::loopback(&server.base_url()).map_err(|_| Error::Connection)?;
    let password = std::iter::once('A')
        .chain((0_u8..16).map(|index| char::from(b'a' + index)))
        .chain(std::iter::once('!'))
        .collect();
    client
        .login(&LoginRequest {
            username: "pager".into(),
            password,
        })
        .await
        .map_err(|_| Error::Connection)?;
    for order in [HistoryOrder::Asc, HistoryOrder::Desc] {
        let mut cursor = None;
        let mut observed = Vec::new();
        loop {
            let response = client
                .messages(
                    Uuid::parse_str(CHANNEL).map_err(|_| Error::Configuration)?,
                    &HistoryQuery {
                        cursor: cursor.as_ref(),
                        order,
                        limit: Some(100),
                    },
                )
                .await
                .map_err(|_| Error::Connection)?;
            assert_eq!(response.channel_id.as_deref(), Some(CHANNEL));
            let messages = response.messages.ok_or(Error::Connection)?;
            cursor = messages.last().and_then(papo_api::wire::Message::cursor);
            observed.extend(messages.into_iter().map(|message| message.id.unwrap()));
            if response.has_more != Some(true) {
                break;
            }
        }
        let mut expected = expected.clone();
        if order == HistoryOrder::Desc {
            expected.reverse();
        }
        assert_eq!(observed, expected);
    }
    let channels: Value = account.get("/channels").await?;
    assert_eq!(channels["channels"][0]["last_read_message"], Value::Null);
    server.shutdown().await
}

#[tokio::test]
async fn history_and_pins_scope_own_reactions_without_inventing_pin_fields() -> Result<(), Error> {
    let server = TestServer::start(Config {
        history: vec![seed(2)],
        ..Config::default()
    })
    .await?;
    let first = Account::register(&server, "first").await?;
    let second = Account::register(&server, "second").await?;
    let path = format!("/channels/{CHANNEL}/messages");
    let messages: MessageListResponse = first.get(&path).await?;
    let id = messages.messages.ok_or(Error::Connection)?[0]
        .id
        .clone()
        .ok_or(Error::Connection)?;
    let unicode = char::from_u32(0x1f44d)
        .ok_or(Error::Configuration)?
        .to_string();
    server
        .set_message_reaction(
            &id,
            MessageReactionSummary {
                unicode: Some(unicode.clone()),
                count: Some(1),
                ..MessageReactionSummary::default()
            },
        )
        .await?;
    server
        .set_user_reactions(
            &first.id,
            &id,
            vec![MessageUserReaction {
                id: Some("80000000-0000-4000-8000-000000000001".into()),
                unicode: Some(unicode),
                ..MessageUserReaction::default()
            }],
        )
        .await?;
    server.set_message_pin(&id, Some(AT)).await?;
    for route in ["messages", "pinned"] {
        let path = format!("/channels/{CHANNEL}/{route}");
        let key = if route == "pinned" {
            "pinned"
        } else {
            "messages"
        };
        let first_body: Value = first.get(&path).await?;
        let second_body: Value = second.get(&path).await?;
        assert_eq!(
            first_body[key][0]["user_reactions"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            second_body[key][0]["user_reactions"]
                .as_array()
                .map(Vec::len),
            Some(0)
        );
        assert!(
            first_body[key][0]["reactions"][0]
                .get("emoji_id")
                .is_some_and(Value::is_null)
        );
        assert!(
            first_body[key][0]["user_reactions"][0]
                .get("emoji_id")
                .is_some_and(Value::is_null)
        );
        assert_eq!(first_body[key][0]["reactions"][0]["count"], 1);
        assert!(first_body[key][0].get("pinned").is_none());
        assert!(first_body[key][0].get("pinned_at").is_none());
    }
    server.shutdown().await
}

#[tokio::test]
async fn history_and_pins_return_backend_auth_statuses_with_channel_not_found_precedence()
-> Result<(), Error> {
    let server = TestServer::start(Config {
        history: vec![seed(1)],
        ..Config::default()
    })
    .await?;
    let first = Account::register(&server, "first").await?;
    let second = Account::register(&server, "second").await?;
    for route in ["messages", "pinned"] {
        let path = format!("/channels/{CHANNEL}/{route}");
        let anonymous = first
            .http
            .get(format!("{}{path}", first.base))
            .send()
            .await
            .map_err(|_| Error::Connection)?;
        assert_eq!(anonymous.status().as_u16(), 401);
    }
    server
        .set_channel_readable(&second.id, CHANNEL, false)
        .await?;
    for route in ["messages", "pinned"] {
        let path = format!("/channels/{CHANNEL}/{route}");
        let response = second
            .http
            .get(format!("{}{path}", second.base))
            .header("Cookie", &second.cookie)
            .send()
            .await
            .map_err(|_| Error::Connection)?;
        assert_eq!(response.status().as_u16(), 403);
        let body: Value = response.json().await.map_err(|_| Error::Connection)?;
        assert!(
            body["type"]
                .as_str()
                .is_some_and(|value| value.ends_with("/errors/forbidden"))
        );
    }
    let invalid_cursor = format!(
        "{}/channels/{CHANNEL}/messages?since={AT}&last_id=invalid",
        second.base
    );
    let response = second
        .http
        .get(invalid_cursor)
        .header("Cookie", &second.cookie)
        .send()
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(response.status().as_u16(), 403);
    let _: PinnedMessageList = first.get(&format!("/channels/{CHANNEL}/pinned")).await?;
    assert!(server.delete_channel(CHANNEL).await);
    for route in ["messages", "pinned"] {
        let response = second
            .http
            .get(format!("{}/channels/{CHANNEL}/{route}", second.base))
            .header("Cookie", &second.cookie)
            .send()
            .await
            .map_err(|_| Error::Connection)?;
        assert_eq!(response.status().as_u16(), 404);
    }
    server.shutdown().await
}

#[tokio::test]
async fn invalid_queries_empty_pins_and_null_fields_match_backend_shapes() -> Result<(), Error> {
    let server = TestServer::start(Config {
        history: vec![seed(1)],
        ..Config::default()
    })
    .await?;
    let account = Account::register(&server, "shape").await?;
    let path = format!("/channels/{CHANNEL}/messages");
    let body: Value = account.get(&path).await?;
    for key in ["content", "edited_at", "reply_to", "embeds", "reactions"] {
        assert!(body["messages"][0].get(key).is_some_and(Value::is_null));
    }
    assert_eq!(
        body["messages"][0]["attachments"].as_array().map(Vec::len),
        Some(0)
    );
    assert_eq!(
        body["messages"][0]["user_reactions"]
            .as_array()
            .map(Vec::len),
        Some(0)
    );
    assert_eq!(body["has_more"], false);
    let pins: PinnedMessageList = account.get(&format!("/channels/{CHANNEL}/pinned")).await?;
    assert_eq!(pins.pinned.ok_or(Error::Connection)?.len(), 0);
    for query in ["limit=0", "limit=101", "order=ASC", "since=bad"] {
        let response = account
            .http
            .get(format!("{}{path}?{query}", account.base))
            .header("Cookie", &account.cookie)
            .send()
            .await
            .map_err(|_| Error::Connection)?;
        assert_eq!(response.status().as_u16(), 400);
    }
    server.shutdown().await
}
