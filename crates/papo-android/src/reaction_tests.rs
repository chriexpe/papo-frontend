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
    pinned: Arc<AtomicBool>,
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
        let pinned=Arc::new(AtomicBool::new(false));
        let current_pin=Arc::clone(&pinned);
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
                let (status, body) = if first.contains("/channels/media/messages/remote-hit/pin") {
                    seen.lock().unwrap().push((format!("PIN-{method}"),Value::Null));
                    if reject.swap(false,Ordering::SeqCst) {("403 Forbidden",r#"{"status":403,"detail":"not allowed"}"#)}
                    else if method=="POST" {current_pin.store(true,Ordering::SeqCst);("200 OK",r#"{"message_id":"remote-hit"}"#)}
                    else if current_pin.swap(false,Ordering::SeqCst) {("204 No Content","")}
                    else {("404 Not Found",r#"{"status":404}"#)}
                } else if first.contains("/channels/media/pinned") {
                    seen.lock().unwrap().push(("PINS".into(),Value::Null));
                    if reject.swap(false,Ordering::SeqCst) {("503 Unavailable",r#"{"status":503,"detail":"try again"}"#)}
                    else if current_pin.load(Ordering::SeqCst) {("200 OK",r#"{"pinned":[{"id":"remote-hit","channel_id":"media","author_id":"ana","content":"full remote message","created_at":"2026-10-20T12:00:00Z","attachments":[{"id":"remote-attachment","mime_type":"image/png","moderation_status":"clean"}],"embeds":[{"id":"remote-embed","url":"https://cdn.test/image.png","image":{"mime_type":"image/png","width":640,"height":480}}]}]}"#)}
                    else {("200 OK",r#"{"pinned":[]}"#)}
                } else if first.contains("/channels/media/messages?") {
                    seen.lock().unwrap().push(("PREVIEW".into(),json!({"request":first.trim()})));
                    if first.contains("order=desc") {("200 OK",r#"{"messages":[],"has_more":false}"#)}
                    else {("200 OK",r#"{"messages":[{"id":"remote-hit","channel_id":"media","author_id":"ana","content":"full remote message","created_at":"2026-10-20T12:00:00Z","attachments":[{"id":"remote-attachment","mime_type":"image/png","moderation_status":"clean"}],"embeds":[{"id":"remote-embed","url":"https://cdn.test/image.png","image":{"mime_type":"image/png","width":640,"height":480}}]}],"has_more":false}"#)}
                } else if first.contains("/search") {
                    seen.lock()
                        .unwrap()
                        .push(("SEARCH".into(), serde_json::from_slice(&bytes).unwrap()));
                    (
                        "200 OK",
                        r#"{"results":[{"type":"message","id":"remote-hit","content":"hello","channel_id":"media","channel_name":"mídia","author_id":"ana","created_at":"2026-10-20T12:00:00Z"}],"has_more":false}"#,
                    )
                } else if method == "PATCH" && first.contains("/server ") {
                    seen.lock()
                        .unwrap()
                        .push(("PATCH".into(), serde_json::from_slice(&bytes).unwrap()));
                    (
                        "200 OK",
                        r#"{"id":"demo","name":"Changed","public":true,"owner_id":"me"}"#,
                    )
                } else if reaction {
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
            pinned,
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

#[test]
fn live_search_uses_resolved_filters_and_discards_a_cleared_request() {
    let fixture = Fixture::new();
    let mut client = fixture.client();
    client
        .dispatch(Action::Search {
            query: "de:@ana em:#mídia tem:link tem:anexo".into(),
            more: false,
        })
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while client.runtime.store.searching {
        client.poll();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(client.runtime.store.search_results[0].id, "remote-hit");
    let requests = fixture.requests.lock().unwrap();
    let body = &requests.iter().find(|r| r.0 == "SEARCH").unwrap().1;
    assert_eq!(body["author"], "ana");
    assert_eq!(body["channel_id"], "media");
    assert_eq!(body["has"], "link");
    assert_eq!(body["contains_attachment"], true);
    drop(requests);
    client
        .dispatch(Action::Search {
            query: "hello".into(),
            more: false,
        })
        .unwrap();
    client.dispatch(Action::ClearSearch).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while fixture
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|r| r.0 == "SEARCH")
        .count()
        < 2
    {
        client.poll();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    client.poll();
    assert!(client.runtime.store.search_results.is_empty());
    assert!(client.search_query.is_empty());
}

#[test]
fn live_server_edits_are_permission_gated_and_wait_for_confirmation() {
    let fixture = Fixture::new();
    let mut client = fixture.client();
    client.runtime.store.me = "ana".into();
    assert!(
        client
            .dispatch(Action::SaveServer {
                name: "Changed".into(),
                public: true,
                password: None
            })
            .is_err()
    );
    assert!(
        !fixture
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.0 == "PATCH")
    );
    client.runtime.store.me = "me".into();
    client
        .dispatch(Action::SaveServer {
            name: "Changed".into(),
            public: true,
            password: None,
        })
        .unwrap();
    assert!(client.runtime.store.busy);
    assert_ne!(
        client.runtime.store.server.as_ref().unwrap().name,
        "Changed"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while client.runtime.store.busy {
        client.poll();
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(
        client.runtime.store.server.as_ref().unwrap().name,
        "Changed"
    );
    let requests = fixture.requests.lock().unwrap();
    let body = &requests.iter().find(|r| r.0 == "PATCH").unwrap().1;
    assert_eq!(body["name"], "Changed");
    assert!(body.get("password").is_none());
}

#[test]
fn catch_up_loads_the_missing_prefix_and_never_navigates_to_the_read_anchor() {
    let fixture = Fixture::new();
    let mut client = fixture.client();
    let wire:Vec<papo_core::api::models::Message>=client.runtime.store.messages_in("general").map(|m|serde_json::from_value(json!({
        "id":m.id,"channel_id":m.channel_id,"author_id":m.author_id,"content":m.content,"created_at":m.at.to_rfc3339()
    })).unwrap()).collect();
    client
        .runtime
        .store
        .observe_chat_viewport("general", &["general-0".into()], false);
    client.runtime.store.reset_read_span("general");
    client
        .runtime
        .store
        .observe_chat_viewport("general", &["general-9".into()], true);
    client.runtime.store.messages.retain(|message| {
        message.channel_id != "general" || wire[5..].iter().any(|m| m.id == message.id)
    });
    let ticket = client.runtime.store.mark_loading("general");
    client.runtime.store.apply(Update::Messages {
        ticket,
        messages: wire[5..].to_vec(),
        has_more: true,
        pinned_ids: None,
    });
    client
        .dispatch(Action::CatchUp {
            channel_id: "general".into(),
            owner_user_id: "me".into(),
        })
        .unwrap();
    assert!(
        client.snapshot()["reading"]["seekingCatchUp"]
            .as_bool()
            .unwrap()
    );
    client.advance_catch_up();
    assert!(
        client.navigation_target.is_empty(),
        "Do not navigate before the prefix is resolved"
    );
    assert!(client.runtime.store.loading_older("general"));
    let ticket = client.runtime.store.history_ticket("general").unwrap();
    client.runtime.store.apply(Update::OlderMessages {
        ticket,
        messages: wire[..5].to_vec(),
        has_more: false,
    });
    client.advance_catch_up();
    assert_eq!(client.navigation_target, "general-1");
    assert_eq!(client.snapshot()["reading"]["gapStart"], "general-1");
    assert!(
        !client.snapshot()["reading"]["seekingCatchUp"]
            .as_bool()
            .unwrap()
    );
}

#[test]
fn leaving_a_catch_up_seek_cancels_future_pagination_without_erasing_the_block() {
    let fixture = Fixture::new();
    let mut client = fixture.client();
    client
        .runtime
        .store
        .observe_chat_viewport("general", &["general-0".into()], false);
    client.runtime.store.reset_read_span("general");
    client
        .runtime
        .store
        .observe_chat_viewport("general", &["general-9".into()], true);
    let before = client.runtime.store.hourglass_range("general");
    client
        .dispatch(Action::CatchUp {
            channel_id: "general".into(),
            owner_user_id: "me".into(),
        })
        .unwrap();
    client
        .dispatch(Action::CancelCatchUp {
            channel_id: "general".into(),
        })
        .unwrap();
    client.advance_catch_up();
    assert!(client.navigation_target.is_empty());
    assert_eq!(client.runtime.store.hourglass_range("general"), before);
}

#[test]
fn asynchronous_attachment_checks_do_not_block_available_media_but_blocked_files_do() {
    let fixture=Fixture::new();let mut client=fixture.client();
    client.runtime.net.session.set_token(Some("fixture".into()));
    let message=client.runtime.store.messages.iter_mut().find(|m|m.channel_id=="general").unwrap();
    message.attachments=vec![serde_json::from_value(json!({"id":"pending-file","mime_type":"image/png","moderation_status":"pending"})).unwrap()];
    for status in ["pending","processing","failed","clean","sensitive"] {
        client.runtime.store.messages.iter_mut().find(|m|!m.attachments.is_empty()).unwrap().attachments[0].moderation_status=Some(status.into());
        assert!(media::request(&client.runtime,media::MediaResource::Attachment {id:"pending-file".into(),thumbnail:false}).is_ok(),"{status} describes the check, not authorization");
    }
    client.runtime.store.messages.iter_mut().find(|m|!m.attachments.is_empty()).unwrap().attachments[0].moderation_status=Some("blocked".into());
    assert!(media::request(&client.runtime,media::MediaResource::Attachment {id:"pending-file".into(),thumbnail:false}).is_err());
}

#[test]
fn result_hydration_and_acknowledged_pins_share_detached_media_without_reading_the_channel() {
    use crate::bindings::{Backend, NativeClient};
    let fixture=Fixture::new();let mut client=fixture.client();
    client.runtime.net.session.set_token(Some("test-session".into()));
    client.runtime.store.search_results=serde_json::from_value(json!([{"id":"remote-hit","channel_id":"media","created_at":"2026-10-20T12:00:00Z"}])).unwrap();
    let native=NativeClient {backend:Mutex::new(Some(Backend::Live(Box::new(client))))};
    assert!(native.load_result_message("another-account".into(),"media".into(),"remote-hit".into()).is_err());
    native.load_result_message("me".into(),"media".into(),"remote-hit".into()).unwrap();
    let snapshot=native.poll().unwrap();assert_eq!(snapshot.selected_channel,"general");
    let result=snapshot.search_results[0].message.as_ref().unwrap();
    assert_eq!(result.content,"full remote message");assert_eq!(result.attachments[0].id,"remote-attachment");assert_eq!(result.embeds[0].id,"remote-embed");
    assert!(!snapshot.messages.iter().any(|m|m.id=="remote-hit"));
    assert!(native.media_request(media::MediaResource::Attachment {id:"remote-attachment".into(),thumbnail:false}).is_ok());
    {
        let backend=native.backend.lock().unwrap();let Some(Backend::Live(client))=backend.as_ref() else {panic!()};
        assert!(client.runtime.store.pending_read_receipts().is_empty());assert!(!client.runtime.store.browsing_history("media"));
        media::check_embed(&client.runtime,"remote-embed").unwrap();
    }
    let requests=fixture.requests.lock().unwrap().iter().filter(|r|r.0=="PREVIEW").count();
    native.load_result_message("me".into(),"media".into(),"remote-hit".into()).unwrap();
    assert_eq!(requests,fixture.requests.lock().unwrap().iter().filter(|r|r.0=="PREVIEW").count());
    native.set_pin("me".into(),"media".into(),"remote-hit".into(),true).unwrap();
    native.load_pins("me".into(),"media".into()).unwrap();
    fixture.fail.store(true,Ordering::SeqCst);
    assert!(native.set_pin("me".into(),"media".into(),"remote-hit".into(),false).is_err());
    assert!(native.poll().unwrap().search_results[0].message.as_ref().unwrap().pinned);
    fixture.fail.store(true,Ordering::SeqCst);
    assert!(native.load_pins("me".into(),"media".into()).is_err());
    {
        let backend=native.backend.lock().unwrap();let Some(Backend::Live(client))=backend.as_ref() else {panic!()};
        assert_eq!(client.runtime.store.pinned_in("media").len(),1);assert!(!client.runtime.store.loading_pinned("media"));
    }
    fixture.pinned.store(false,Ordering::SeqCst);
    native.load_pins("me".into(),"media".into()).unwrap();
    assert!(!native.poll().unwrap().search_results[0].message.as_ref().unwrap().pinned);
    native.set_pin("me".into(),"media".into(),"remote-hit".into(),true).unwrap();
    fixture.pinned.store(false,Ordering::SeqCst); // A concurrent participant unpinned it first.
    native.set_pin("me".into(),"media".into(),"remote-hit".into(),false).unwrap();
    assert!(!native.poll().unwrap().search_results[0].message.as_ref().unwrap().pinned);
    {
        let mut backend=native.backend.lock().unwrap();let Some(Backend::Live(client))=backend.as_mut() else {panic!()};
        client.runtime.store.server.as_mut().unwrap().owner_id=Some("someone-else".into());client.runtime.store.roles.clear();
    }
    let before=fixture.requests.lock().unwrap().len();
    assert!(native.set_pin("me".into(),"media".into(),"remote-hit".into(),true).is_err());
    assert_eq!(before,fixture.requests.lock().unwrap().len());
    native.shutdown();
}

#[test]
fn visible_pins_refresh_unknown_live_events_and_keep_events_during_inflight_snapshots() {
    use crate::bindings::{Backend, NativeClient};
    let fixture = Fixture::new();
    let client = fixture.client();
    client.runtime.net.session.set_token(Some("test-session".into()));
    let native = NativeClient { backend: Mutex::new(Some(Backend::Live(Box::new(client)))) };
    assert!(native.watch_pins("wrong-account".into(), "media".into(), true).is_err());
    native.watch_pins("me".into(), "media".into(), true).unwrap();
    fixture.pinned.store(true, Ordering::SeqCst);
    {
        let mut backend = native.backend.lock().unwrap();
        let Some(Backend::Live(client)) = backend.as_mut() else { panic!() };
        assert!(client.runtime.store.preview_message("remote-hit").is_none());
        let ticket = client.runtime.store.begin_load_pinned("media", false).unwrap();
        let event = Update::Event(Box::new(papo_core::api::ws::Event::MessagePinned {
            message_id: "remote-hit".into(), pinned: true,
        }));
        client.observe_pin_update(&event);
        client.runtime.store.apply(event);
        client.refresh_watched_pins();
        assert!(client.watched_pins_dirty, "Do not lose events behind an older in-flight request");
        assert!(fixture.requests.lock().unwrap().is_empty());
        client.runtime.store.apply(Update::PinnedMessages { ticket: Some(ticket), channel_id: "media".into(), messages: vec![] });
        client.refresh_watched_pins();
        assert!(!client.watched_pins_dirty);
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        native.poll().unwrap();
        let backend = native.backend.lock().unwrap();
        let Some(Backend::Live(client)) = backend.as_ref() else { panic!() };
        if !client.runtime.store.loading_pinned("media") { break; }
        assert!(std::time::Instant::now() < deadline);
        drop(backend);
        std::thread::sleep(Duration::from_millis(2));
    }
    {
        let backend = native.backend.lock().unwrap();
        let Some(Backend::Live(client)) = backend.as_ref() else { panic!() };
        assert_eq!(client.runtime.store.pinned_in("media")[0].id, "remote-hit");
        assert_eq!(client.runtime.store.selected_channel, "general");
        assert!(client.runtime.store.pending_read_receipts().is_empty());
    }
    native.watch_pins("me".into(), "general".into(), false).unwrap();
    { let backend = native.backend.lock().unwrap(); let Some(Backend::Live(client)) = backend.as_ref() else { panic!() }; assert!(client.watched_pins.is_some()); }
    native.watch_pins("me".into(), "media".into(), false).unwrap();
    { let mut backend = native.backend.lock().unwrap(); let Some(Backend::Live(client)) = backend.as_mut() else { panic!() }; client.watched_pins_dirty = true; client.refresh_watched_pins(); assert!(!client.watched_pins_dirty); assert!(!client.runtime.store.loading_pinned("media")); }
    native.shutdown();
}
