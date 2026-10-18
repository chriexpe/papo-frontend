use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

const AT: &str = "2026-10-17T12:00:00.123456789Z";

async fn mock_history(
    expected_requests: usize,
    deleted: bool,
) -> (Api, tokio::task::JoinHandle<()>, Arc<AtomicUsize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let requests = count.clone();
    let task = tokio::spawn(async move {
        for _ in 0..expected_requests {
            let (stream, _) = listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            let mut line = String::new();
            stream.read_line(&mut line).await.unwrap();
            let path = line.split_whitespace().nth(1).unwrap();
            let url = Url::parse(&format!("http://{address}{path}")).unwrap();
            // No new backend endpoint is needed for a context jump.
            assert_eq!(url.path(), "/channels/chat/messages");
            let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
            let ascending = query.get("order").is_some_and(|order| order == "asc");
            let limit = query["limit"].parse::<usize>().unwrap();
            let cursor = query.get("since").map(|at| {
                assert_eq!(at, AT, "the timestamp must not lose fractional precision");
                query["last_id"].clone()
            });
            let mut messages: Vec<_> = (0..100)
                .filter(|index| !deleted || *index != 50)
                .map(|index| format!("m{index:03}"))
                .filter(|id| {
                    cursor
                        .as_ref()
                        .is_none_or(|cursor| if ascending { id > cursor } else { id < cursor })
                })
                .collect();
            if !ascending {
                messages.reverse();
            }
            let has_more = messages.len() > limit;
            messages.truncate(limit);
            let messages: Vec<_> = messages.iter().map(|id| serde_json::json!({
                "id": id, "channel_id": "chat", "author_id": "other", "content": "fresh from server",
                "created_at": AT, "attachments": [{"id": "gif", "mime_type": "image/gif", "original_file_name": "animated.gif"}],
            })).collect();
            loop {
                line.clear();
                if stream.read_line(&mut line).await.unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            let body = serde_json::json!({"channel_id": "chat", "messages": messages, "has_more": has_more}).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .get_mut()
                .write_all(response.as_bytes())
                .await
                .unwrap();
            requests.fetch_add(1, Ordering::Relaxed);
        }
    });
    (
        Api::new(&format!("http://{address}"), Arc::new(Session::default())).unwrap(),
        task,
        count,
    )
}

#[tokio::test]
async fn context_uses_two_existing_requests_and_rehydrates_target_with_equal_timestamps() {
    let (api, server, count) = mock_history(2, false).await;
    let context = api
        .message_context("chat", "m050", Some(AT.parse().unwrap()))
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 2);
    assert_eq!(context.messages.len(), 61);
    assert_eq!(context.messages.first().unwrap().id, "m020");
    assert_eq!(context.messages.last().unwrap().id, "m080");
    let target = context
        .messages
        .iter()
        .find(|message| message.id == "m050")
        .unwrap();
    assert_eq!(target.content.as_deref(), Some("fresh from server"));
    assert_eq!(target.attachments[0].id, "gif");
    assert!(context.has_more_before && context.has_more_after);
}

#[tokio::test]
async fn context_can_open_the_first_message_without_a_predecessor() {
    let (api, server, _) = mock_history(2, false).await;
    let context = api
        .message_context("chat", "m000", Some(AT.parse().unwrap()))
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(context.messages.first().unwrap().id, "m000");
    assert!(!context.has_more_before);
    assert!(context.has_more_after);
}

#[tokio::test]
async fn context_last_message_has_no_newer_page() {
    let (api, server, _) = mock_history(2, false).await;
    let context = api
        .message_context("chat", "m099", Some(AT.parse().unwrap()))
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(context.messages.last().unwrap().id, "m099");
    assert!(!context.has_more_after);
}

#[tokio::test]
async fn opaque_reply_id_is_located_without_projecting_intermediate_pages() {
    let (api, server, count) = mock_history(3, false).await;
    let context = api.message_context("chat", "m050", None).await.unwrap();
    server.await.unwrap();
    assert_eq!(count.load(Ordering::Relaxed), 3);
    assert_eq!(context.messages.first().unwrap().id, "m020");
}

#[tokio::test]
async fn deleted_target_is_not_reconstructed_from_stale_pin_or_search_text() {
    let (api, server, _) = mock_history(2, true).await;
    assert!(
        api.message_context("chat", "m050", Some(AT.parse().unwrap()))
            .await
            .is_err()
    );
    server.await.unwrap();
}
