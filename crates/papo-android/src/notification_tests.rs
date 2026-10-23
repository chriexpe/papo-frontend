//! Real HTTP fixtures for paginated inbox and independent preference ACKs.
use super::*;
use crate::bindings::{Backend, NativeClient};
use std::{
    io::{BufRead, Read, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Instant,
};

type Inspect = Box<dyn FnOnce(&str, &Value) + Send>;

struct Reply {
    method: &'static str,
    path: &'static str,
    status: u16,
    body: Value,
    inspect: Inspect,
    gate: Option<mpsc::Receiver<()>>,
}
struct Fixture {
    url: String,
    replies: Arc<Mutex<std::collections::VecDeque<Reply>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let replies = Arc::new(Mutex::new(std::collections::VecDeque::<Reply>::new()));
        let queue = Arc::clone(&replies);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                let Ok((stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream);
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
                    if let Some((key, value)) = line.split_once(':') {
                        if key.eq_ignore_ascii_case("content-length") {
                            length = value.trim().parse().unwrap();
                        }
                        if key.eq_ignore_ascii_case("cookie") {
                            cookie = value.trim().into();
                        }
                    }
                }
                let mut bytes = vec![0; length];
                reader.read_exact(&mut bytes).unwrap();
                let reply = queue.lock().unwrap().pop_front();
                let (status, body) = if let Some(reply) = reply {
                    let path = first.split_whitespace().nth(1).unwrap();
                    assert!(first.starts_with(reply.method), "{first}");
                    assert_eq!(path.split('?').next().unwrap(), reply.path);
                    assert_eq!(cookie, "Auth=fixture");
                    (reply.inspect)(path, &serde_json::from_slice(&bytes).unwrap_or(Value::Null));
                    if let Some(gate) = reply.gate {
                        gate.recv_timeout(Duration::from_secs(5)).unwrap();
                    }
                    (reply.status, reply.body.to_string())
                } else {
                    (401, "{}".into())
                };
                write!(reader.get_mut(), "HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nRetry-After: 0\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let path =
            std::env::temp_dir().join(format!("papo-inbox-{}", papo_core::cache::new_local_id()));
        std::fs::create_dir(&path).unwrap();
        Self {
            url,
            replies,
            stop,
            worker: Some(worker),
            path,
        }
    }
    fn reply(
        &self,
        method: &'static str,
        path: &'static str,
        status: u16,
        body: Value,
        inspect: impl FnOnce(&str, &Value) + Send + 'static,
    ) {
        self.replies.lock().unwrap().push_back(Reply {
            method,
            path,
            status,
            body,
            inspect: Box::new(inspect),
            gate: None,
        });
    }
    fn client(&self) -> Arc<NativeClient> {
        let mut client = Client::open(
            &self.url,
            self.path.join("client.db"),
            Arc::new(papo_core::storage::MemorySecretStore::default()),
            Wake::new(|| {}),
        )
        .unwrap();
        let until = Instant::now() + Duration::from_secs(5);
        while client.runtime.store.screen != Screen::Auth {
            client.poll();
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        client.runtime.store = std::mem::take(&mut demo::Demo::new().store);
        client.runtime.net.session.set_token(Some("fixture".into()));
        Arc::new(NativeClient {
            backend: Mutex::new(Some(Backend::Live(Box::new(client)))),
        })
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
fn notice(id: &str, fraction: &str, read: bool) -> Value {
    json!({"id":id,"channel_id":"general","author_id":"ana","message_id":format!("message-{id}"),"created_at":format!("2026-10-23T23:00:00.{fraction}Z"),"read":read,"message_content":"hello"})
}
#[test]
fn bulk_preferences_ack_each_channel_preserve_failures_and_retry_429_through_the_governor() {
    let fixture = Fixture::new();
    let client = fixture.client();
    for (path, status) in [
        ("/channels/general/user/me/settings", 200),
        ("/channels/design/user/me/settings", 403),
        ("/channels/dev/user/me/settings", 429),
        ("/channels/dev/user/me/settings", 200),
    ] {
        fixture.reply("POST", path, status, json!({}), |_, body| {
            assert_eq!(*body, json!({"notification_settings":"all"}))
        });
    }
    let result = client
        .notification_policy(
            "me".into(),
            vec![
                "general".into(),
                "design".into(),
                "dev".into(),
                "missing".into(),
                "general".into(),
            ],
            "all".into(),
        )
        .unwrap();
    assert_eq!(result.applied, ["general", "dev"]);
    assert_eq!(result.failed.len(), 2);
    assert_eq!(result.failed[0].reason, "denied");
    assert_eq!(result.failed[1].reason, "missing");
    let snapshot = client.poll().unwrap();
    assert_eq!(
        snapshot
            .channels
            .iter()
            .find(|c| c.id == "general")
            .unwrap()
            .notification_settings,
        "all"
    );
    assert_eq!(
        snapshot
            .channels
            .iter()
            .find(|c| c.id == "design")
            .unwrap()
            .notification_settings,
        "only_mentions"
    );
    // A local value is not an ACK: another client may have changed it.
    for path in [
        "/channels/general/user/me/settings",
        "/channels/dev/user/me/settings",
    ] {
        fixture.reply("POST", path, 200, json!({}), |_, body| {
            assert_eq!(*body, json!({"notification_settings":"all"}))
        });
    }
    let repeated = client
        .notification_policy(
            "me".into(),
            vec!["general".into(), "dev".into()],
            "all".into(),
        )
        .unwrap();
    assert_eq!(repeated.applied.len(), 2);
    assert!(
        client
            .notification_policy("other".into(), vec!["general".into()], "off".into())
            .is_err()
    );
    assert!(
        client
            .notification_policy("me".into(), vec!["general".into()], "invalid".into())
            .is_err()
    );
    assert!(fixture.replies.lock().unwrap().is_empty());
}
#[test]
fn exact_cursor_and_read_ack_survive_older_pages_and_live_head_refreshes() {
    let fixture = Fixture::new();
    let client = fixture.client();
    fixture.reply("GET", "/users/me/notifications", 200, json!({"notifications":[notice("n2","000000009",false),notice("n1","000000001",false)],"has_more":true}), |path,_| assert!(!path.contains('?')));
    let head = client.inbox_page("me".into(), false).unwrap();
    assert_eq!(head.items.len(), 2);
    assert!(head.has_more);
    fixture.reply(
        "PUT",
        "/users/me/read_notification",
        200,
        json!({"updated":1}),
        |_, body| assert_eq!(*body, json!({"notification_ids":["n1"]})),
    );
    client.inbox_read("me".into(), vec!["n1".into()]).unwrap();
    fixture.reply("GET", "/users/me/notifications", 200, json!({"notifications":[notice("n1","000000001",false),notice("n0","000000000",true)],"has_more":false}), |path,_| {
        let url = url::Url::parse(&format!("http://fixture{path}")).unwrap();
        let params: HashMap<_,_> = url.query_pairs().into_owned().collect();
        let at = chrono::DateTime::parse_from_rfc3339(&params["since"]).unwrap();
        assert_eq!(at.timestamp_subsec_nanos(), 1);
        assert_eq!(params["last_id"], "n1"); assert_eq!(params["order"], "desc");
    });
    let older = client.inbox_page("me".into(), true).unwrap();
    assert!(!older.has_more);
    assert_eq!(older.items.len(), 3);
    assert!(older.items.iter().find(|n| n.id == "n1").unwrap().read);
    fixture.reply("GET", "/users/me/notifications", 200, json!({"notifications":[notice("n3","000000010",false),notice("n2","000000009",false)],"has_more":true}), |_,_| {});
    let refreshed = client.inbox_page("me".into(), false).unwrap();
    assert_eq!(refreshed.items.len(), 4);
    assert!(!refreshed.has_more);
    assert!(refreshed.items.iter().find(|n| n.id == "n1").unwrap().read);
    {
        let mut backend = client.backend.lock().unwrap();
        let Some(Backend::Live(c)) = backend.as_mut() else {
            panic!()
        };
        c.runtime
            .net
            .session
            .set_token(Some("renewed-fixture".into()));
    }
    assert_eq!(
        client.inbox_snapshot("me".into()).unwrap().items.len(),
        4,
        "credential renewal must retain this owner's loaded pages"
    );
    assert!(client.inbox_page("other".into(), false).is_err());
    assert!(
        client
            .inbox_read("me".into(), vec!["x".into(); 1001])
            .is_err()
    );
    assert!(fixture.replies.lock().unwrap().is_empty());
}
#[test]
fn concurrent_pages_are_rejected_and_old_accounts_cannot_publish_a_response() {
    let fixture = Fixture::new();
    let client = fixture.client();
    let (entered, waiting) = mpsc::channel();
    let (release, gate) = mpsc::channel();
    fixture.replies.lock().unwrap().push_back(Reply {
        method: "GET",
        path: "/users/me/notifications",
        status: 200,
        body: json!({"notifications":[notice("stale","000000001",false)],"has_more":false}),
        inspect: Box::new(move |_, _| {
            entered.send(()).unwrap();
        }),
        gate: Some(gate),
    });
    let other = Arc::clone(&client);
    let thread = std::thread::spawn(move || other.inbox_page("me".into(), false));
    waiting.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        client.inbox_page("me".into(), false).is_err(),
        "no duplicate request"
    );
    {
        let mut backend = client.backend.lock().unwrap();
        let Some(Backend::Live(c)) = backend.as_mut() else {
            panic!()
        };
        c.runtime.store.me = "other".into();
        c.runtime.net.session.set_token(Some("other-token".into()));
    }
    assert!(
        client
            .inbox_snapshot("other".into())
            .unwrap()
            .items
            .is_empty()
    );
    release.send(()).unwrap();
    assert!(thread.join().unwrap().is_err());
    assert!(
        client
            .inbox_snapshot("other".into())
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn presentation_filtering_cannot_erase_a_raw_cursor() {
    let client = NativeClient::demo();
    {
        let mut backend = client.backend.lock().unwrap();
        let Some(Backend::Demo(c)) = backend.as_mut() else {
            panic!()
        };
        c.notifications = (0..101).map(|i| {
            serde_json::from_value(json!({"id":format!("n{i:03}"),"created_at":format!("2026-10-23T23:00:00.{:09}Z", 200-i),"author_id":if i==100 { "ana" } else { "me" },"read":false})).unwrap()
        }).collect();
    }
    let first = client.inbox_page("me".into(), false).unwrap();
    assert!(first.items.is_empty());
    assert!(first.has_more);
    assert_eq!(first.cursor.unwrap().last_id, "n099");
    let more = client.inbox_page("me".into(), true).unwrap();
    assert_eq!(more.items.len(), 1);
    assert!(!more.has_more);
    assert_eq!(more.items[0].id, "n100");
}
