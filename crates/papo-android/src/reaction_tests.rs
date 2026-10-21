//! Exercises the live adapter and real core HTTP worker against a loopback
//! fixture. Demo reduction alone cannot catch stale own-reaction flags.
use super::*;
use std::io::{BufRead, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};

struct Fixture {
    url: String,
    requests: Arc<Mutex<Vec<(String, Value)>>>,
    fail: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Fixture {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let fail = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let (seen, reject, stop) = (
            Arc::clone(&requests),
            Arc::clone(&fail),
            Arc::clone(&stopped),
        );
        let thread = std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some((key, value)) = line.split_once(':')
                        && key.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let method = first
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .to_owned();
                let reaction = first.contains("/reactions ");
                let (status, body) = if reaction {
                    seen.lock()
                        .unwrap()
                        .push((method.clone(), serde_json::from_slice(&bytes).unwrap()));
                    if reject.swap(false, Ordering::SeqCst) {
                        ("403 Forbidden", r#"{"status":403,"detail":"not allowed"}"#)
                    } else if method == "DELETE" {
                        ("204 No Content", "")
                    } else {
                        ("200 OK", r#"{"id":"reaction"}"#)
                    }
                } else {
                    ("401 Unauthorized", r#"{"status":401}"#)
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        });
        Self {
            url,
            requests,
            fail,
            stopped,
            thread: Some(thread),
        }
    }
    fn client(&self) -> Client {
        let path = std::env::temp_dir().join(format!(
            "papo-reactions-{}.db",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut client = Client::open(
            &self.url,
            path,
            Arc::new(papo_core::storage::MemorySecretStore::default()),
            Wake::new(|| {}),
        )
        .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while client.runtime.store.screen != Screen::Auth {
            client.poll();
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut demo = demo::Demo::new();
        client.runtime.store = std::mem::take(&mut demo.store);
        client
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
fn settle(client: &mut Client) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !client.pending_reactions.is_empty() {
        client.poll();
        assert!(
            std::time::Instant::now() < deadline,
            "reaction did not settle"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn react(client: &mut Client, id: &str, emoji: &Emoji, add: bool) {
    client
        .dispatch(Action::React {
            channel_id: "general".into(),
            message_id: id.into(),
            emoji: match emoji {
                Emoji::Unicode(e) | Emoji::Custom(e) => e.clone(),
            },
            custom: matches!(emoji, Emoji::Custom(_)),
            add,
        })
        .unwrap();
}

#[test]
fn live_add_remove_add_updates_mine_and_rolls_back_rejections() {
    let fixture = Fixture::new();
    let mut client = fixture.client();
    let id = client
        .runtime
        .store
        .messages_in("general")
        .last()
        .unwrap()
        .id
        .clone();
    for emoji in [
        Emoji::Unicode("👍".into()),
        Emoji::Custom("demo-wave".into()),
    ] {
        client
            .runtime
            .store
            .messages
            .iter_mut()
            .find(|m| m.id == id)
            .unwrap()
            .reactions
            .push(papo_core::state::Reaction {
                emoji: emoji.clone(),
                count: 1,
                mine: false,
            });
        for add in [true, false, true] {
            react(&mut client, &id, &emoji, add);
            settle(&mut client);
            let r = client
                .runtime
                .store
                .message(&id)
                .unwrap()
                .reactions
                .iter()
                .find(|r| r.emoji == emoji)
                .unwrap();
            assert_eq!(r.mine, add);
            assert_eq!(r.count, if add { 2 } else { 1 });
        }
        fixture.fail.store(true, Ordering::SeqCst);
        react(&mut client, &id, &emoji, false);
        settle(&mut client);
        let r = client
            .runtime
            .store
            .message(&id)
            .unwrap()
            .reactions
            .iter()
            .find(|r| r.emoji == emoji)
            .unwrap();
        assert!(r.mine);
        assert_eq!(r.count, 2);
    }
    let seen = fixture.requests.lock().unwrap();
    assert_eq!(
        seen.iter().map(|r| r.0.as_str()).collect::<Vec<_>>(),
        [
            "POST", "DELETE", "POST", "DELETE", "POST", "DELETE", "POST", "DELETE"
        ]
    );
    assert_eq!(seen[1].1, json!({"unicode":"👍"}));
    assert_eq!(seen[5].1, json!({"emoji_id":"demo-wave"}));
}
#[test]
fn rapid_taps_coalesce_without_overlapping_reaction_requests() {
    let fixture = Fixture::new();
    let mut client = fixture.client();
    let id = client
        .runtime
        .store
        .messages_in("general")
        .last()
        .unwrap()
        .id
        .clone();
    let emoji = Emoji::Unicode("🎉".into());
    for add in [true, false, true] {
        react(&mut client, &id, &emoji, add);
    }
    settle(&mut client);
    assert_eq!(fixture.requests.lock().unwrap().len(), 1);
    react(&mut client, &id, &emoji, false);
    react(&mut client, &id, &emoji, true);
    settle(&mut client);
    assert_eq!(
        fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.0.as_str())
            .collect::<Vec<_>>(),
        ["POST", "DELETE", "POST"]
    );
    assert!(
        client
            .runtime
            .store
            .message(&id)
            .unwrap()
            .reactions
            .iter()
            .any(|r| r.emoji == emoji && r.mine && r.count == 1)
    );
}

#[test]
fn rejection_restores_own_flag_without_subtracting_an_authoritative_count_twice() {
    let mut demo = demo::Demo::new();
    let store = &mut demo.store;
    let id = store.messages_in("general").last().unwrap().id.clone();
    let emoji = Emoji::Unicode("👍".into());
    store.set_reaction_local(&id, &emoji, true);
    store.apply(Update::Event(Box::new(
        papo_core::api::ws::Event::Reaction {
            message_id: id.clone(),
            unicode: Some("👍".into()),
            emoji_id: None,
            count: 2,
        },
    )));
    store.restore_reaction_local(&id, &emoji, false, 2);
    let r = store
        .message(&id)
        .unwrap()
        .reactions
        .iter()
        .find(|r| r.emoji == emoji)
        .unwrap();
    assert!(!r.mine);
    assert_eq!(r.count, 2);
}

#[test]
fn successful_http_reinstates_own_flag_after_an_older_websocket_count_removed_the_group() {
    let fixture = Fixture::new();
    let mut client = fixture.client();
    let id = client
        .runtime
        .store
        .messages_in("general")
        .last()
        .unwrap()
        .id
        .clone();
    let emoji = Emoji::Unicode("🎉".into());
    react(&mut client, &id, &emoji, true);
    client.runtime.store.apply(Update::Event(Box::new(
        papo_core::api::ws::Event::Reaction {
            message_id: id.clone(),
            unicode: Some("🎉".into()),
            emoji_id: None,
            count: 0,
        },
    )));
    settle(&mut client);
    assert!(
        client
            .runtime
            .store
            .message(&id)
            .unwrap()
            .reactions
            .iter()
            .any(|r| r.emoji == emoji && r.mine && r.count == 1)
    );
}
