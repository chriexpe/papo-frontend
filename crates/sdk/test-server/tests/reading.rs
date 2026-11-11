//! Owns TCP read-mark contract proofs; leaves pure ordering and upload policy tests outside.

mod test_client;

use papo_test_server::{Config, Error, Failure, HistorySeed, RestStep, TestServer};
use serde_json::{Value, json};
use test_client::Account;

const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
const FIRST: &str = "20000000-0000-4000-8000-000000000001";

fn config() -> Config {
    Config {
        history: vec![HistorySeed {
            channel_id: CHANNEL.into(),
            author_id: "30000000-0000-4000-8000-000000000001".into(),
            first_message_id: FIRST.into(),
            count: 3,
            first_created_at: "2026-01-01T00:00:00.123456Z".into(),
            timestamp_step_us: 0,
            equal_timestamp_count: 3,
        }],
        ..Config::default()
    }
}

#[tokio::test]
async fn explicit_read_returns_empty_204_and_lists_account_mark() -> Result<(), Error> {
    let server = TestServer::start(config()).await?;
    let account = Account::register(&server, "reader").await?;
    let response = account
        .http
        .put(format!("{}/channels/{CHANNEL}/read", account.base))
        .header("Cookie", &account.cookie)
        .json(&json!({"message_id": FIRST}))
        .send()
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(response.status().as_u16(), 204);
    assert!(
        response
            .bytes()
            .await
            .map_err(|_| Error::Connection)?
            .is_empty()
    );
    let channels: Value = account.get("/channels").await?;
    assert_eq!(
        channels["channels"][0]["last_message"]["id"],
        "20000000-0000-4000-8000-000000000003"
    );
    assert_eq!(channels["channels"][0]["last_read_message"], FIRST);
    assert!(channels["channels"][0]["last_read_at"].is_string());
    server.shutdown().await
}

const SECOND: &str = "20000000-0000-4000-8000-000000000002";
const ACK: &str = "2026-02-01T00:00:00.654321Z";

async fn put(account: &Account, channel: &str, body: Value) -> Result<reqwest::Response, Error> {
    account
        .http
        .put(format!("{}/channels/{channel}/read", account.base))
        .header("Cookie", &account.cookie)
        .json(&body)
        .send()
        .await
        .map_err(|_| Error::Connection)
}

#[tokio::test]
async fn remote_mark_is_account_scoped_and_has_no_read_event() -> Result<(), Error> {
    let server = TestServer::start(config()).await?;
    let first = Account::register(&server, "reader").await?;
    let second = Account::register(&server, "other").await?;
    let mut socket = first.socket(&server).await?;
    assert_eq!(
        test_client::next(&mut socket).await?.kind(),
        "presence_sync"
    );
    assert!(
        server
            .advance_read_mark(&first.id, CHANNEL, SECOND, ACK)
            .await?
    );
    assert!(
        !server
            .advance_read_mark(&first.id, CHANNEL, FIRST, "2027-01-01T00:00:00Z")
            .await?
    );
    // The next real event is a barrier: no synthetic read event may precede it.
    assert!(server.rename_channel(CHANNEL, "renamed").await);
    assert_eq!(
        test_client::resource_event(&mut socket).await?.kind(),
        "channel_update"
    );
    let channels: Value = first.get("/channels").await?;
    assert_eq!(channels["channels"][0]["last_read_message"], SECOND);
    assert_eq!(channels["channels"][0]["last_read_at"], ACK);
    let channels: Value = second.get("/channels").await?;
    assert!(channels["channels"][0]["last_read_message"].is_null());
    drop(socket);
    server.shutdown().await
}

#[tokio::test]
async fn old_and_duplicate_http_reads_keep_the_previous_acknowledgement() -> Result<(), Error> {
    let server = TestServer::start(config()).await?;
    let account = Account::register(&server, "reader").await?;
    server.set_read_acknowledged_at(ACK).await?;
    assert_eq!(
        put(&account, CHANNEL, json!({"message_id": SECOND}))
            .await?
            .status()
            .as_u16(),
        204
    );
    server
        .set_read_acknowledged_at("2027-01-01T00:00:00Z")
        .await?;
    for id in [FIRST, SECOND] {
        assert_eq!(
            put(&account, CHANNEL, json!({"message_id": id}))
                .await?
                .status()
                .as_u16(),
            204
        );
    }
    let channels: Value = account.get("/channels").await?;
    assert_eq!(channels["channels"][0]["last_read_at"], ACK);
    assert_eq!(channels["channels"][0]["last_read_message"], SECOND);
    server.shutdown().await
}

#[tokio::test]
async fn history_fetch_does_not_acknowledge_and_read_errors_leave_no_mark() -> Result<(), Error> {
    let server = TestServer::start(config()).await?;
    let account = Account::register(&server, "reader").await?;
    let _: Value = account
        .get(&format!("/channels/{CHANNEL}/messages"))
        .await?;
    let response = account
        .http
        .put(format!("{}/channels/{CHANNEL}/read", account.base))
        .json(&json!({"message_id": FIRST}))
        .send()
        .await
        .map_err(|_| Error::Connection)?;
    assert_eq!(response.status().as_u16(), 401);
    for body in [
        json!({}),
        json!({"message_id": ""}),
        json!({"message_id": 3}),
    ] {
        assert_eq!(put(&account, CHANNEL, body).await?.status().as_u16(), 400);
    }
    for (channel, id, status) in [
        (CHANNEL, "20000000-0000-4000-8000-000000000099", 404),
        ("10000000-0000-4000-8000-000000000099", FIRST, 404),
        (CHANNEL, "malformed", 500),
    ] {
        let response = put(&account, channel, json!({"message_id": id})).await?;
        assert_eq!(response.status().as_u16(), status);
        assert_eq!(
            response.headers()["content-type"],
            "application/problem+json"
        );
    }
    let other = server.create_channel("other").await?;
    assert_eq!(
        put(
            &account,
            other.id.as_deref().ok_or(Error::Configuration)?,
            json!({"message_id": FIRST})
        )
        .await?
        .status()
        .as_u16(),
        404
    );
    server
        .set_channel_readable(&account.id, CHANNEL, false)
        .await?;
    assert_eq!(
        put(&account, CHANNEL, json!({"message_id": FIRST}))
            .await?
            .status()
            .as_u16(),
        403
    );
    let channels: Value = account.get("/channels").await?;
    assert!(channels["channels"][0]["last_read_message"].is_null());
    server.shutdown().await
}

#[tokio::test]
async fn failures_retry_and_held_uploads_are_counted_without_retaining_bodies() -> Result<(), Error>
{
    let mut config = config();
    config.rest = vec![503, 404]
        .into_iter()
        .map(|status| RestStep {
            method: "PUT".into(),
            path: format!("/channels/{CHANNEL}/read"),
            delay_ms: 0,
            failure: Some(Failure {
                status,
                slug: "not-found".into(),
            }),
        })
        .collect();
    let server = TestServer::start(config).await?;
    let account = Account::register(&server, "reader").await?;
    for status in [503, 404] {
        assert_eq!(
            put(&account, CHANNEL, json!({"message_id": FIRST}))
                .await?
                .status()
                .as_u16(),
            status
        );
    }
    let mut hold = server
        .hold_next_rest("PUT", &format!("/channels/{CHANNEL}/read"))
        .await?;
    let uploading =
        tokio::spawn(async move { put(&account, CHANNEL, json!({"message_id": FIRST})).await });
    tokio::time::timeout(std::time::Duration::from_secs(5), hold.wait_started())
        .await
        .map_err(|_| Error::Connection)??;
    let counts = server.read_request_counts().await;
    assert_eq!(
        (counts.uploads, counts.inflight, counts.max_inflight),
        (3, 1, 1)
    );
    hold.release();
    assert_eq!(
        uploading.await.map_err(|_| Error::Task)??.status().as_u16(),
        204
    );
    assert_eq!(server.read_request_counts().await.inflight, 0);
    server.shutdown().await
}
