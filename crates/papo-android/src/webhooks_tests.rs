use super::*;
use crate::{Client, Wake, demo::Demo};
use serde_json::{Value, json};
use std::{
    io::{BufRead, Read, Write},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

#[test]
fn webhook_only_delegates_can_manage_hooks_but_manage_server_alone_is_not_enough() {
    let native = NativeClient::demo();
    {
        let mut backend = native.backend.lock().unwrap();
        let Some(Backend::Demo(demo)) = backend.as_mut() else {
            panic!()
        };
        demo.store.server.as_mut().unwrap().owner_id = Some("owner".into());
        demo.store.roles = vec![
            serde_json::from_value(
                json!({"id":"hooks","name":"Integrations","permissions":{"manage_server":true}}),
            )
            .unwrap(),
        ];
        demo.store
            .members
            .iter_mut()
            .find(|m| m.id == "me")
            .unwrap()
            .roles = vec!["hooks".into()];
    }
    assert!(matches!(
        native.webhooks("me".into()),
        Err(WebhookError::Denied)
    ));
    {
        let mut backend = native.backend.lock().unwrap();
        let Some(Backend::Demo(demo)) = backend.as_mut() else {
            panic!()
        };
        demo.store.roles[0].permissions.manage_server = false;
        demo.store.roles[0].permissions.manage_webhooks = true;
    }
    assert!(native.administration("me".into(), false).is_ok());
    assert!(native.webhooks("me".into()).unwrap().is_empty());
    assert!(matches!(
        native.create_webhook("other-owner".into(), "general".into(), "Hook".into()),
        Err(WebhookError::AccountChanged)
    ));
    assert!(matches!(
        native.create_webhook("me".into(), "projects".into(), "Hook".into()),
        Err(WebhookError::InvalidChannel)
    ));
    assert!(matches!(
        native.create_webhook("me".into(), "dm-ana".into(), "Hook".into()),
        Err(WebhookError::InvalidChannel)
    ));
    assert!(matches!(
        native.create_webhook("me".into(), "general".into(), "😀".repeat(81)),
        Err(WebhookError::InvalidName)
    ));
    let receipt = native
        .create_webhook("me".into(), "general".into(), "😀".repeat(80))
        .unwrap();
    let id = receipt.webhook().id;
    let token = receipt.take_token();
    assert!(!token.is_empty());
    assert!(
        receipt.take_token().is_empty(),
        "native credential can only be consumed once"
    );
    assert!(!format!("{:?}", native.webhooks("me".into()).unwrap()).contains(&token));
    {
        let backend = native.backend.lock().unwrap();
        let Some(Backend::Demo(demo)) = backend.as_ref() else {
            panic!()
        };
        assert!(
            !crate::projection::project(&demo.store, "demo://papo", "demo")
                .to_string()
                .contains(&token)
        );
    }
    assert!(matches!(
        native.update_webhook("me".into(), id.clone(), "x".repeat(101), None, None),
        Err(WebhookError::InvalidName)
    ));
    assert!(matches!(
        native.update_webhook(
            "me".into(),
            id.clone(),
            "Nick".into(),
            Some("bad-image".into()),
            Some("PNG".into())
        ),
        Err(WebhookError::InvalidAvatar)
    ));
    native
        .update_webhook("me".into(), id.clone(), "Nick".into(), None, None)
        .unwrap();
    assert_eq!(
        native.webhooks("me".into()).unwrap()[0].display_name,
        "Nick"
    );
    native.revoke_webhook("me".into(), id).unwrap();
    assert!(native.webhooks("me".into()).unwrap().is_empty());
}

#[test]
fn created_credentials_are_redacted_and_patch_omission_differs_from_clearing() {
    let raw = json!({"id":"hook","channel_id":"general","name":"Hook","created_by":"me","created_at":"2026-10-23T12:00:00Z","token":"test-only-secret"});
    let created: models::CreatedWebhook = serde_json::from_value(raw).unwrap();
    assert!(!format!("{created:?}").contains("test-only-secret"));
    assert!(
        serde_json::to_value(&created.webhook)
            .unwrap()
            .get("token")
            .is_none()
    );
    let preserve = models::UpdateWebhookIdentity {
        display_name: Some("Nick".into()),
        ..Default::default()
    };
    assert_eq!(
        serde_json::to_value(preserve).unwrap(),
        json!({"display_name":"Nick"})
    );
    let clear = models::UpdateWebhookIdentity {
        display_name: Some("".into()),
        avatar: Some("".into()),
        avatar_format: None,
    };
    assert_eq!(
        serde_json::to_value(clear).unwrap(),
        json!({"display_name":"","avatar":""})
    );
}

struct Reply {
    method: &'static str,
    path: &'static str,
    status: u16,
    body: Value,
    expected: Value,
}
struct Fixture {
    url: String,
    queue: Arc<Mutex<std::collections::VecDeque<Reply>>>,
    stop: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
    directory: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let queue = Arc::new(Mutex::new(std::collections::VecDeque::<Reply>::new()));
        let replies = Arc::clone(&queue);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(2));
                    continue;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let mut length = 0;
                let mut cookie = String::new();
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line.is_empty() || line == "\r\n" {
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
                let response = replies.lock().unwrap().pop_front();
                let (status, body) = if let Some(response) = response {
                    assert!(
                        first.starts_with(&format!("{} {} ", response.method, response.path)),
                        "unexpected method/path"
                    );
                    assert_eq!(cookie, "Auth=fixture");
                    assert_eq!(
                        serde_json::from_slice::<Value>(&bytes).unwrap_or_default(),
                        response.expected
                    );
                    (
                        response.status,
                        if response.status == 204 {
                            String::new()
                        } else {
                            response.body.to_string()
                        },
                    )
                } else {
                    (401, "{\"status\":401}".into())
                };
                write!(stream, "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let directory =
            std::env::temp_dir().join(format!("papo-webhook-{}", papo_core::cache::new_local_id()));
        std::fs::create_dir(&directory).unwrap();
        Self {
            url,
            queue,
            stop,
            worker: Some(worker),
            directory,
        }
    }
    fn client(&self) -> NativeClient {
        let mut live = Client::open(
            &self.url,
            self.directory.join("chat.db"),
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
        NativeClient {
            backend: Mutex::new(Some(Backend::Live(Box::new(live)))),
        }
    }
    fn expect(
        &self,
        method: &'static str,
        path: &'static str,
        status: u16,
        body: Value,
        expected: Value,
    ) {
        self.queue.lock().unwrap().push_back(Reply {
            method,
            path,
            status,
            body,
            expected,
        });
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.worker.take().unwrap().join().unwrap();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

#[test]
fn live_requests_preserve_identity_fields_and_return_semantic_failures_without_retrying_creation() {
    let fixture = Fixture::new();
    let native = fixture.client();
    let hook = json!({"id":"hook","channel_id":"general","name":"Builds","created_by":"me","created_at":"2026-10-23T12:00:00Z"});
    fixture.expect("GET", "/webhooks", 403, json!({"status":403}), Value::Null);
    assert!(matches!(
        native.webhooks("me".into()),
        Err(WebhookError::Denied)
    ));
    fixture.expect(
        "GET",
        "/webhooks",
        200,
        json!({"webhooks":[hook.clone()]}),
        Value::Null,
    );
    assert_eq!(native.webhooks("me".into()).unwrap()[0].name, "Builds");
    fixture.expect(
        "POST",
        "/webhooks",
        200,
        json!({"unexpected":"response"}),
        json!({"channel_id":"general","name":"Builds"}),
    );
    assert!(matches!(
        native.create_webhook("me".into(), "general".into(), "Builds".into()),
        Err(WebhookError::CreationUnconfirmed)
    ));
    let mut created = hook.clone();
    created["token"] = json!("test-only-secret");
    fixture.expect(
        "POST",
        "/webhooks",
        201,
        created,
        json!({"channel_id":"general","name":"Builds"}),
    );
    let receipt = native
        .create_webhook("me".into(), "general".into(), "Builds".into())
        .unwrap();
    assert_eq!(receipt.take_token(), "test-only-secret");
    assert!(receipt.take_token().is_empty());
    let mut updated = hook.clone();
    updated["display_name"] = json!("News");
    fixture.expect(
        "PATCH",
        "/webhooks/hook",
        200,
        updated,
        json!({"display_name":"News"}),
    );
    assert_eq!(
        native
            .update_webhook("me".into(), "hook".into(), "News".into(), None, None)
            .unwrap()
            .display_name,
        "News"
    );
    use base64::Engine;
    let avatar = base64::engine::general_purpose::STANDARD.encode(include_bytes!(
        "../../../android/compose/src/main/res/mipmap-mdpi/ic_launcher.png"
    ));
    fixture.expect(
        "PATCH",
        "/webhooks/hook",
        200,
        hook.clone(),
        json!({"display_name":"News","avatar":avatar,"avatar_format":"PNG"}),
    );
    native
        .update_webhook(
            "me".into(),
            "hook".into(),
            "News".into(),
            Some(avatar),
            Some("PNG".into()),
        )
        .unwrap();
    fixture.expect(
        "PATCH",
        "/webhooks/hook",
        200,
        hook.clone(),
        json!({"display_name":"","avatar":""}),
    );
    assert!(
        native
            .update_webhook("me".into(), "hook".into(), "".into(), Some("".into()), None)
            .unwrap()
            .display_name
            .is_empty()
    );
    fixture.expect(
        "DELETE",
        "/webhooks/hook",
        403,
        json!({"status":403}),
        Value::Null,
    );
    assert!(matches!(
        native.revoke_webhook("me".into(), "hook".into()),
        Err(WebhookError::Denied)
    ));
    fixture.expect("DELETE", "/webhooks/hook", 204, Value::Null, Value::Null);
    native.revoke_webhook("me".into(), "hook".into()).unwrap();
    fixture.expect("GET", "/webhooks", 401, json!({"status":401}), Value::Null);
    assert!(matches!(
        native.webhooks("me".into()),
        Err(WebhookError::SignedOut)
    ));
    assert!(fixture.queue.lock().unwrap().is_empty());
    let request = native.scoped_api("me").unwrap();
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
    assert!(native.finish_request(&request, vec![]).is_err());
    native.shutdown();
}
