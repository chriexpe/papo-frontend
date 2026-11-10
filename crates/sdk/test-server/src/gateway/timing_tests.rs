//! Owns deterministic gateway timing over in-memory duplex streams; leaves TCP integration and wall clocks outside.

use super::*;
use tokio::sync::broadcast;

#[tokio::test(start_paused = true)]
async fn script_delays_keep_heartbeats_responsive_without_real_tcp_or_clock() {
    let (server, client) = tokio::io::duplex(16 * 1024);
    let mut presence = crate::presence::Presence::new();
    let (snapshot, updates) = presence.join("member");
    let (disconnects, disconnected) = broadcast::channel(1);
    let start = tokio::time::Instant::now();
    let task = tokio::spawn(async move {
        serve(
            server,
            snapshot,
            updates,
            &[
                GatewayStep::Delay { delay_ms: 25 },
                GatewayStep::Event {
                    event: json!({"type": "heartbeat_ack"}),
                },
                GatewayStep::Delay { delay_ms: 25 },
                GatewayStep::Disconnect,
            ],
            false,
            disconnected,
            "member",
        )
        .await;
    });
    let mut socket = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    let sync = socket.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&sync.into_data()).unwrap()["type"],
        "presence_sync"
    );
    socket
        .send(Message::text(json!({"type": "heartbeat"}).to_string()))
        .await
        .unwrap();
    let acknowledgement = socket.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&acknowledgement.into_data()).unwrap()["type"],
        "heartbeat_ack"
    );
    assert_eq!(tokio::time::Instant::now() - start, Duration::ZERO);
    let scripted = socket.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&scripted.into_data()).unwrap()["type"],
        "heartbeat_ack"
    );
    assert_eq!(
        tokio::time::Instant::now() - start,
        Duration::from_millis(25)
    );
    assert!(socket.next().await.unwrap().unwrap().is_close());
    assert_eq!(
        tokio::time::Instant::now() - start,
        Duration::from_millis(50)
    );
    task.await.unwrap();
    drop(disconnects);
}

#[tokio::test(start_paused = true)]
async fn live_updates_are_delivered_during_script_delay_and_filter_account_recipients() {
    let (server, client) = tokio::io::duplex(16 * 1024);
    let mut presence = crate::presence::Presence::new();
    let (snapshot, updates) = presence.join("member");
    let (disconnects, disconnected) = broadcast::channel(1);
    let start = tokio::time::Instant::now();
    let task = tokio::spawn(async move {
        serve(
            server,
            snapshot,
            updates,
            &[GatewayStep::Delay { delay_ms: 100 }],
            false,
            disconnected,
            "member",
        )
        .await;
    });
    let mut socket = WebSocketStream::from_raw_socket(client, Role::Client, None).await;
    assert!(socket.next().await.unwrap().unwrap().is_text());
    presence.emit(
        Some("other"),
        json!({"type":"message_delete", "id":"ignored", "channel_id":"ignored"}),
    );
    presence.emit(Some("member"), json!({"type":"heartbeat_ack"}));
    let frame = socket.next().await.unwrap().unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&frame.into_data()).unwrap()["type"],
        "heartbeat_ack"
    );
    assert_eq!(tokio::time::Instant::now() - start, Duration::ZERO);
    disconnects.send(()).unwrap();
    task.await.unwrap();
}
