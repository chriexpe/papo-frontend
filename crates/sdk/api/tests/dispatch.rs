//! Real socket proof of pre-dispatch versus ambiguous failure; API transport never performs policy retries.

use bytes::Bytes;
use futures_util::stream;
use papo_api::{
    Error, MessageUpload, RestClient, StreamedMessageAttachment, StreamedMessageUpload,
};
use std::io;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn request() -> MessageUpload<'static> {
    MessageUpload {
        channel_id: "10000000-0000-4000-8000-000000000001",
        content: None,
        reply_to: None,
        embeds: None,
        attachments: &[],
    }
}

#[tokio::test]
async fn failed_connection_is_proven_not_dispatched() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let client = RestClient::loopback(&format!("http://{address}")).unwrap();
    assert!(matches!(
        client.create_message(&request()).await,
        Err(Error::NotDispatched)
    ));
}

#[tokio::test]
async fn accepted_bytes_and_lost_response_remain_ambiguous_without_transport_retry() {
    let result = after_acceptance(false).await;
    assert!(matches!(result, Err(Error::Transport)));
}

#[tokio::test]
async fn malformed_success_after_acceptance_is_not_a_safe_pre_dispatch_error() {
    let result = after_acceptance(true).await;
    assert!(matches!(result, Err(Error::Decode)));
}

async fn after_acceptance(malformed_response: bool) -> Result<papo_api::wire::Message, Error> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client =
        RestClient::loopback(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let n = socket.read(&mut buffer).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buffer[..n]);
            assert!(bytes.len() < 65536);
            if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                let header = std::str::from_utf8(&bytes[..end]).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .map(|n| n.trim().parse().unwrap())
                    })
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        // Drop the listener as well: an unsafe hidden retry would become NotDispatched and fail the assertion.
        drop(listener);
        if malformed_response {
            socket.write_all(b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{").await.unwrap();
        }
    });
    let result = client.create_message(&request()).await;
    server.await.unwrap();
    result
}

#[tokio::test]
async fn streamed_upload_failed_connection_is_not_dispatched_and_mid_stream_error_is_transport() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let client = RestClient::loopback(&format!("http://{address}")).unwrap();
    let failed_connect = client
        .create_message_streamed(StreamedMessageUpload {
            channel_id: "10000000-0000-4000-8000-000000000001",
            content: Some("stream"),
            reply_to: None,
            embeds: None,
            attachments: vec![StreamedMessageAttachment {
                filename: "a.bin".into(),
                size_bytes: 4,
                stream: Box::pin(stream::once(std::future::ready(Ok(Bytes::from_static(
                    b"abcd",
                ))))),
            }],
        })
        .await;
    assert!(matches!(failed_connect, Err(Error::NotDispatched)));

    // Mid-stream I/O error after TCP connection must be classified Transport (ambiguous), never NotDispatched.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client =
        RestClient::loopback(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0_u8; 4096];
        while let Ok(n) = socket.read(&mut buf).await {
            if n == 0 {
                break;
            }
        }
    });
    let chunks: Vec<io::Result<Bytes>> = vec![
        Ok(Bytes::from_static(b"first-part")),
        Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "staged file truncated",
        )),
    ];
    let mid_fail = client
        .create_message_streamed(StreamedMessageUpload {
            channel_id: "10000000-0000-4000-8000-000000000001",
            content: Some("stream"),
            reply_to: None,
            embeds: None,
            attachments: vec![StreamedMessageAttachment {
                filename: "broken.bin".into(),
                size_bytes: 64,
                stream: Box::pin(stream::iter(chunks)),
            }],
        })
        .await;
    server.await.unwrap();
    assert!(matches!(mid_fail, Err(Error::Transport)));
}
