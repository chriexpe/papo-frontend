use super::*;
use crate::{bindings::Backend, demo::Demo};
use papo_core::api::net::Update;
use serde_json::json;

#[test]
fn hiding_blocking_and_reopening_preserve_history_and_account_isolation() {
    let client = NativeClient::demo();
    let other = NativeClient::demo();
    client
        .open_direct_message("me".into(), "ana".into())
        .unwrap();
    let dm = client.poll().unwrap().selected_channel;
    client
        .dispatch(crate::Action::Send {
            embeds: vec![],
            local_id: "kept-message".into(),
            channel_id: dm.clone(),
            content: "Keep this history".into(),
            reply_to: None,
            notify_reply: false,
        })
        .unwrap();
    assert!(
        client
            .hide_direct_message("wrong-owner".into(), dm.clone())
            .is_err()
    );
    assert!(
        client
            .set_person_blocked("me".into(), "me".into(), true)
            .is_err()
    );
    assert_eq!(client.poll().unwrap().dms.len(), 1);
    client.hide_direct_message("me".into(), dm.clone()).unwrap();
    let hidden = client.poll().unwrap();
    assert!(hidden.dms.is_empty());
    assert!(hidden.selected_channel.is_empty());
    client
        .open_direct_message("me".into(), "ana".into())
        .unwrap();
    assert!(
        client
            .poll()
            .unwrap()
            .messages
            .iter()
            .any(|m| m.id == "kept-message")
    );
    client
        .set_person_blocked("me".into(), "ana".into(), true)
        .unwrap();
    assert!(client.poll().unwrap().dms.is_empty());
    assert_eq!(client.blocked_people("me".into()).unwrap()[0].name, "Ana");
    assert!(matches!(
        client.open_direct_message("me".into(), "ana".into()),
        Err(SocialError::Blocked)
    ));
    assert!(other.poll().unwrap().blocked_users.is_empty());
    client
        .set_person_blocked("me".into(), "ana".into(), false)
        .unwrap();
    assert!(
        client.poll().unwrap().dms.is_empty(),
        "unblocking does not reopen a DM"
    );
    client
        .open_direct_message("me".into(), "ana".into())
        .unwrap();
    let reopened = client.poll().unwrap();
    assert_eq!(reopened.selected_channel, dm);
    assert!(reopened.messages.iter().any(|m| m.id == "kept-message"));
}

#[test]
fn a_late_dm_snapshot_or_event_cannot_resurrect_a_blocked_conversation() {
    let mut demo = Demo::new();
    demo.dispatch(crate::Action::OpenDm {
        user_id: "ana".into(),
    })
    .unwrap();
    let mut dm = demo.store.direct_messages[0].clone();
    dm.unread_count = 3;
    demo.store.apply(Update::DirectMessages(vec![dm.clone()]));
    assert_eq!(demo.store.direct_unread_total(), 3);
    demo.store.apply(Update::UserBlockChanged {
        user_id: "ana".into(),
        blocked: true,
    });
    demo.store.apply(Update::DirectMessages(vec![dm.clone()]));
    demo.store.apply(Update::Event(Box::new(
        papo_core::api::ws::Event::DirectMessageUpdated(Box::new(dm.clone())),
    )));
    demo.store.apply(Update::DirectMessageOpened(Box::new(dm)));
    assert!(demo.store.direct_messages.is_empty());
    assert!(demo.store.channel("dm-ana").is_none());
    assert_eq!(demo.store.direct_unread_total(), 0);
}

#[test]
fn live_social_requests_are_acknowledged_and_use_the_canonical_core_contract() {
    use crate::{Client, Wake};
    use std::{
        io::{BufRead, Read, Write},
        sync::{Arc, Mutex},
        time::{Duration, Instant},
    };
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let script = Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let queue = Arc::clone(&script);
    let stopped = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    let worker = std::thread::spawn(move || {
        while !stop.load(std::sync::atomic::Ordering::SeqCst) {
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
            let mut cookie = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some((k, v)) = line.split_once(':') {
                    if k.eq_ignore_ascii_case("content-length") {
                        length = v.trim().parse().unwrap();
                    }
                    if k.eq_ignore_ascii_case("cookie") {
                        cookie = v.trim().into();
                    }
                }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            let next = queue.lock().unwrap().pop_front();
            let (status, body) = if let Some((method, path, status, response, expected_body)) = next
            {
                assert!(first.starts_with(&format!("{method} {path} ")), "{first}");
                assert_eq!(cookie, "Auth=fixture");
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default(),
                    expected_body
                );
                (status, response)
            } else {
                (401, "{\"status\":401}")
            };
            write!(stream, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        }
    });
    let path = std::env::temp_dir().join(format!(
        "papo-social-{}.db",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let mut live = Client::open(
        &url,
        path,
        Arc::new(papo_core::storage::MemorySecretStore::default()),
        Wake::new(|| {}),
    )
    .unwrap();
    let until = Instant::now() + Duration::from_secs(5);
    while live.runtime.store.screen != papo_core::state::Screen::Auth {
        live.poll();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    live.runtime.store = std::mem::take(&mut Demo::new().store);
    live.runtime.net.session.set_token(Some("fixture".into()));
    let native = NativeClient {
        backend: Mutex::new(Some(Backend::Live(Box::new(live)))),
        previews: Default::default(),
    };
    let dm = r#"{"id":"dm-ana","user":{"id":"ana","username":"ana","nickname":"Ana"},"created_at":"2026-10-23T12:00:00Z","unread_count":2}"#;
    script.lock().unwrap().extend([
        ("POST", "/dms", 200, dm, json!({"user_id":"ana"})),
        (
            "DELETE",
            "/dms/dm-ana",
            403,
            "{\"status\":403}",
            serde_json::Value::Null,
        ),
        ("DELETE", "/dms/dm-ana", 204, "", serde_json::Value::Null),
        ("POST", "/dms", 200, dm, json!({"user_id":"ana"})),
        (
            "POST",
            "/users/ana/block",
            403,
            "{\"status\":403}",
            serde_json::Value::Null,
        ),
        ("POST", "/users/ana/block", 204, "", serde_json::Value::Null),
        (
            "GET",
            "/users/blocks",
            200,
            "{\"users\":[{\"id\":\"ana\",\"username\":\"ana\",\"nickname\":\"Ana\"}]}",
            serde_json::Value::Null,
        ),
        (
            "DELETE",
            "/users/ana/block",
            204,
            "",
            serde_json::Value::Null,
        ),
        (
            "POST",
            "/dms",
            403,
            "{\"type\":\"dm-blocked\",\"status\":403}",
            json!({"user_id":"ana"}),
        ),
        ("POST", "/dms", 200, dm, json!({"user_id":"ana"})),
        (
            "POST",
            "/dms",
            403,
            "{\"type\":\"dm-blocked\",\"status\":403}",
            json!({"user_id":"ana"}),
        ),
    ]);
    assert!(
        native
            .open_direct_message("wrong-owner".into(), "ana".into())
            .is_err()
    );
    native
        .open_direct_message("me".into(), "ana".into())
        .unwrap();
    assert!(
        native
            .hide_direct_message("me".into(), "dm-ana".into())
            .is_err()
    );
    // Inspect the store directly: polling would start unrelated background hydration.
    let visible = || {
        let backend = native.backend.lock().unwrap();
        let Some(Backend::Live(c)) = backend.as_ref() else {
            panic!()
        };
        c.runtime.store.direct_messages.len()
    };
    assert_eq!(visible(), 1);
    native
        .hide_direct_message("me".into(), "dm-ana".into())
        .unwrap();
    assert_eq!(visible(), 0);
    native
        .open_direct_message("me".into(), "ana".into())
        .unwrap();
    assert!(
        native
            .set_person_blocked("me".into(), "ana".into(), true)
            .is_err()
    );
    assert_eq!(visible(), 1);
    native
        .set_person_blocked("me".into(), "ana".into(), true)
        .unwrap();
    assert_eq!(visible(), 0);
    assert_eq!(
        native.blocked_people("me".into()).unwrap()[0].username,
        "ana"
    );
    native
        .set_person_blocked("me".into(), "ana".into(), false)
        .unwrap();
    assert_eq!(visible(), 0);
    assert!(matches!(
        native.open_direct_message("me".into(), "ana".into()),
        Err(SocialError::Blocked)
    ));
    assert_eq!(visible(), 0);
    native
        .open_direct_message("me".into(), "ana".into())
        .unwrap();
    assert_eq!(visible(), 1);
    assert!(matches!(
        native.open_direct_message("me".into(), "ana".into()),
        Err(SocialError::Blocked)
    ));
    assert_eq!(
        visible(),
        0,
        "a block by the peer removes the inaccessible DM too"
    );
    assert!(script.lock().unwrap().is_empty());
    let old_request = native.scoped_api("me").unwrap();
    {
        let mut backend = native.backend.lock().unwrap();
        let Some(Backend::Live(client)) = backend.as_mut() else {
            panic!()
        };
        client.runtime.store.me = "next-account".into();
        client
            .runtime
            .net
            .session
            .set_token(Some("next-token".into()));
    }
    assert!(
        native
            .finish_request(
                &old_request,
                vec![Update::UserBlockChanged {
                    user_id: "bruno".into(),
                    blocked: true,
                }]
            )
            .is_err()
    );
    {
        let backend = native.backend.lock().unwrap();
        let Some(Backend::Live(client)) = backend.as_ref() else {
            panic!()
        };
        assert!(!client.runtime.store.is_user_blocked("bruno"));
    }
    native.shutdown();
    stopped.store(true, std::sync::atomic::Ordering::SeqCst);
    worker.join().unwrap();
}
