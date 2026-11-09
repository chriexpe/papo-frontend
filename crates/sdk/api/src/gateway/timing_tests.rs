//! Exercises heartbeat deadlines using only in-memory duplex sockets and Tokio's virtual clock.

use super::*;
use futures_util::poll;
use tokio::io::DuplexStream;
use tokio_tungstenite::tungstenite::protocol::Role;

type Peer = WebSocketStream<DuplexStream>;

async fn pair() -> (Gateway<DuplexStream>, Peer) {
    let (client, server) = tokio::io::duplex(4096);
    let (client, server) = tokio::join!(
        WebSocketStream::from_raw_socket(client, Role::Client, None),
        WebSocketStream::from_raw_socket(server, Role::Server, None),
    );
    (Gateway::from_socket(client), server)
}

#[tokio::test(start_paused = true)]
async fn first_heartbeat_waits_25_seconds_and_missing_ack_ends_at_next_deadline() {
    let (mut gateway, mut peer) = pair().await;
    let mut next = Box::pin(gateway.next());
    assert!(poll!(next.as_mut()).is_pending());
    tokio::time::advance(Duration::from_secs(24)).await;
    assert!(poll!(next.as_mut()).is_pending());
    assert!(poll!(peer.next()).is_pending());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(poll!(next.as_mut()).is_pending());
    let frame = peer.next().await.unwrap().unwrap();
    assert!(matches!(frame, Message::Text(text) if text == r#"{"type":"heartbeat"}"#));
    tokio::time::advance(Duration::from_secs(24)).await;
    assert!(poll!(next.as_mut()).is_pending());
    assert!(poll!(peer.next()).is_pending());
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(matches!(next.await, Err(Error::HeartbeatTimeout)));
    assert!(gateway.next().await.unwrap().is_none());
    assert!(matches!(
        gateway.send(&Command::Heartbeat).await,
        Err(Error::Transport)
    ));
    assert!(poll!(peer.next()).is_pending());
}

#[tokio::test(start_paused = true)]
async fn application_ack_allows_next_heartbeat_and_does_not_reconnect_after_timeout() {
    let (mut gateway, mut peer) = pair().await;
    let mut next = Box::pin(gateway.next());
    assert!(poll!(next.as_mut()).is_pending());
    tokio::time::advance(Duration::from_secs(25)).await;
    assert!(poll!(next.as_mut()).is_pending());
    assert!(matches!(
        peer.next().await.unwrap().unwrap(),
        Message::Text(_)
    ));
    peer.send(Message::text(r#"{"type":"heartbeat_ack"}"#))
        .await
        .unwrap();
    assert_eq!(next.await.unwrap().unwrap().kind(), "heartbeat_ack");
    let mut next = Box::pin(gateway.next());
    tokio::time::advance(Duration::from_secs(25)).await;
    assert!(poll!(next.as_mut()).is_pending());
    assert!(matches!(
        peer.next().await.unwrap().unwrap(),
        Message::Text(_)
    ));
    tokio::time::advance(Duration::from_secs(25)).await;
    assert!(matches!(next.await, Err(Error::HeartbeatTimeout)));
    assert!(gateway.next().await.unwrap().is_none());
}

#[tokio::test(start_paused = true)]
async fn protocol_ping_is_answered_but_pong_is_not_an_application_ack() {
    let (mut gateway, mut peer) = pair().await;
    let mut next = Box::pin(gateway.next());
    assert!(poll!(next.as_mut()).is_pending());
    peer.send(Message::Ping(Vec::new().into())).await.unwrap();
    assert!(poll!(next.as_mut()).is_pending());
    assert!(matches!(
        peer.next().await.unwrap().unwrap(),
        Message::Pong(_)
    ));
    tokio::time::advance(Duration::from_secs(25)).await;
    assert!(poll!(next.as_mut()).is_pending());
    assert!(matches!(
        peer.next().await.unwrap().unwrap(),
        Message::Text(_)
    ));
    peer.send(Message::Pong(Vec::new().into())).await.unwrap();
    assert!(poll!(next.as_mut()).is_pending());
    tokio::time::advance(Duration::from_secs(25)).await;
    assert!(matches!(next.await, Err(Error::HeartbeatTimeout)));
}

#[tokio::test(start_paused = true)]
async fn cancelling_a_pending_read_does_not_reset_heartbeat_deadline_or_ack_state() {
    let (mut gateway, mut peer) = pair().await;
    let mut next = Box::pin(gateway.next());
    assert!(poll!(next.as_mut()).is_pending());
    tokio::time::advance(Duration::from_secs(25)).await;
    assert!(poll!(next.as_mut()).is_pending());
    peer.next().await.unwrap().unwrap();
    drop(next);
    let mut next = Box::pin(gateway.next());
    tokio::time::advance(Duration::from_secs(25)).await;
    assert!(matches!(
        poll!(next.as_mut()),
        std::task::Poll::Ready(Err(Error::HeartbeatTimeout))
    ));
}
