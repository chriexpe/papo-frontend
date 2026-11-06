//! Loopback-only backend for desktop lifecycle tests and runtime measurements.
//!
//! Every account, credential and message is synthetic. History uses the real
//! timestamp/ID cursor contract, including nanosecond timestamp ties. REST
//! pages and persistent sockets share one ephemeral loopback listener.
//! Controls replay existing messages (not invented continuity) or close live
//! sockets; counters and a condition variable make integration waits bounded.
//! This is test infrastructure, not another desktop transport implementation.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use futures_util::{SinkExt as _, StreamExt as _};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::broadcast;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::{Message, handshake::derive_accept_key, protocol::Role};

/// Synthetic account returned by the fixture's authentication endpoint.
pub const ACCOUNT_ID: &str = "fixture-account";
/// The fixture's only chronological text channel.
pub const CHANNEL_ID: &str = "fixture-channel";
/// Fake credential, usable only with the ephemeral loopback fixture.
pub const SESSION_TOKEN: &str = "fixture-session";
/// Equal timestamps deliberately exercise the stable ID cursor tie-breaker.
const AT: &str = "2000-01-01T00:00:00.123456789Z";

/// Content-free backend progress for deterministic integration assertions.
#[derive(Clone, Default, Debug)]
pub struct BackendStats {
    /// Total successful WebSocket subscriptions, including reconnects.
    pub connections: usize,
    /// Currently retained WebSocket sessions.
    pub active_sockets: usize,
    /// Successfully transmitted message events, counting replays.
    pub emitted_events: usize,
    /// Parsed HTTP requests, including WebSocket upgrades.
    pub http_requests: usize,
    /// Chronological history page requests.
    pub history_requests: usize,
    /// HTTP responses outside the successful fixture contract.
    pub http_errors: usize,
    /// Unexpected connection-handler failures.
    pub failures: usize,
    /// Last received client event type, never its full payload.
    pub last_client_type: Option<String>,
}

#[derive(Default)]
struct Progress {
    state: Mutex<BackendStats>,
    changed: Condvar,
}

impl Progress {
    fn update(&self, change: impl FnOnce(&mut BackendStats)) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        change(&mut state);
        self.changed.notify_all();
    }
}

#[derive(Clone, Copy)]
enum Control {
    Replay(usize),
    Disconnect,
    Stop,
}

/// One independently owned local backend; dropping it cancels tasks and joins.
pub struct FixtureBackend {
    url: String,
    controls: broadcast::Sender<Control>,
    progress: Arc<Progress>,
    thread: Option<JoinHandle<()>>,
}

impl FixtureBackend {
    /// Starts a loopback backend with `message_count` historical rows.
    /// Each WebSocket subscription replays up to `replay_on_connect` rows.
    ///
    /// # Errors
    /// Returns an I/O error if binding, runtime creation or thread spawn fails.
    pub fn start(message_count: usize, replay_on_connect: usize) -> io::Result<Self> {
        let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        listener.set_nonblocking(true)?;
        let url = format!("http://{}", listener.local_addr()?);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        let (controls, mut receiver) = broadcast::channel(16);
        let progress = Arc::new(Progress::default());
        let worker_progress = Arc::clone(&progress);
        let worker_controls = controls.clone();
        let thread = std::thread::Builder::new().name("desktop-fixture".into()).spawn(move || {
            runtime.block_on(async move {
                let Ok(listener) = TcpListener::from_std(listener) else {
                    worker_progress.update(|state| state.failures += 1);
                    return;
                };
                let mut tasks = tokio::task::JoinSet::new();
                loop {
                    tokio::select! {
                        command = receiver.recv() => {
                            if matches!(command, Ok(Control::Stop) | Err(broadcast::error::RecvError::Closed)) {
                                break;
                            }
                        }
                        accepted = listener.accept() => {
                            let Ok((stream, _)) = accepted else { break };
                            let progress = Arc::clone(&worker_progress);
                            let controls = worker_controls.subscribe();
                            tasks.spawn(async move {
                                if serve(stream, message_count, replay_on_connect, controls, &progress).await.is_err() {
                                    progress.update(|state| state.failures += 1);
                                }
                            });
                        }
                        _ = tasks.join_next(), if !tasks.is_empty() => {}
                    }
                }
                tasks.abort_all();
                while tasks.join_next().await.is_some() {}
            });
        })?;
        Ok(Self {
            url,
            controls,
            progress,
            thread: Some(thread),
        })
    }

    /// The ephemeral HTTP origin to pass to the real `ServerRuntime`.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Replays the latest `count` existing rows on each currently live socket.
    pub fn replay(&self, count: usize) -> bool {
        self.controls.send(Control::Replay(count)).is_ok()
    }

    /// Closes current sockets; the core reconnects normally or after a probe.
    pub fn disconnect(&self) -> bool {
        self.controls.send(Control::Disconnect).is_ok()
    }

    /// Returns a content-free snapshot of backend progress.
    pub fn stats(&self) -> BackendStats {
        self.progress
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Waits without polling until a progress predicate holds or time expires.
    pub fn wait_for(&self, timeout: Duration, ready: impl Fn(&BackendStats) -> bool) -> bool {
        let state = self
            .progress
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.progress
            .changed
            .wait_timeout_while(state, timeout, |state| !ready(state))
            .is_ok_and(|(state, _)| ready(&state))
    }

    /// Cancels accepted connections and joins the server thread; safe to repeat.
    pub fn shutdown(&mut self) -> bool {
        let _ = self.controls.send(Control::Stop);
        self.thread
            .take()
            .is_none_or(|thread| thread.join().is_ok())
    }
}

impl Drop for FixtureBackend {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct ActiveSocket<'a>(&'a Progress);

impl Drop for ActiveSocket<'_> {
    fn drop(&mut self) {
        self.0.update(|state| state.active_sockets -= 1);
    }
}

async fn serve(
    stream: TcpStream,
    total: usize,
    initial_replay: usize,
    mut controls: broadcast::Receiver<Control>,
    progress: &Progress,
) -> io::Result<()> {
    let mut stream = BufReader::new(stream);
    let (method, path, key) = tokio::time::timeout(Duration::from_secs(5), async {
        let mut line = String::new();
        stream.read_line(&mut line).await?;
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_owned();
        let path = parts.next().unwrap_or_default().to_owned();
        let mut key = None;
        let mut length = 0;
        let mut header_bytes = line.len();
        loop {
            line.clear();
            if stream.read_line(&mut line).await? == 0 || line == "\r\n" {
                break;
            }
            header_bytes += line.len();
            if header_bytes > 32_768 {
                return Err(io::Error::other("fixture headers too large"));
            }
            if let Some((name, value)) = line.split_once(':') {
                if name.eq_ignore_ascii_case("sec-websocket-key") {
                    key = Some(value.trim().to_owned());
                }
                if name.eq_ignore_ascii_case("content-length") {
                    length = value.trim().parse::<usize>().map_err(io::Error::other)?;
                }
            }
        }
        if length > 65_536 {
            return Err(io::Error::other("fixture body too large"));
        }
        stream.read_exact(&mut vec![0; length]).await?;
        Ok::<_, io::Error>((method, path, key))
    })
    .await
    .map_err(io::Error::other)??;
    progress.update(|state| state.http_requests += 1);
    if path == "/ws" && method == "GET" {
        let key = key.ok_or_else(|| io::Error::other("missing fixture upgrade key"))?;
        let accept = derive_accept_key(key.as_bytes());
        stream.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes()).await?;
        let mut socket = WebSocketStream::from_raw_socket(stream, Role::Server, None).await;
        progress.update(|state| {
            state.connections += 1;
            state.active_sockets += 1;
        });
        let _active = ActiveSocket(progress);
        replay(&mut socket, total, initial_replay, progress).await?;
        loop {
            tokio::select! {
                control = controls.recv() => match control {
                    Ok(Control::Replay(count)) => replay(&mut socket, total, count, progress).await?,
                    Ok(Control::Disconnect | Control::Stop) | Err(_) => {
                        let _ = socket.close(None).await;
                        return Ok(());
                    }
                },
                incoming = socket.next() => match incoming {
                    Some(Ok(Message::Text(text))) => {
                        let value: Value = serde_json::from_str(&text).map_err(io::Error::other)?;
                        let kind = value["type"].as_str().unwrap_or_default();
                        progress.update(|state| state.last_client_type = Some(kind.to_owned()));
                        if kind == "heartbeat" {
                            socket.send(Message::Text(json!({"type":"heartbeat_ack"}).to_string().into())).await.map_err(io::Error::other)?;
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => socket.send(Message::Pong(payload)).await.map_err(io::Error::other)?,
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return Ok(()),
                    _ => {}
                }
            }
        }
    }
    let url = url::Url::parse(&format!("http://127.0.0.1{path}")).map_err(io::Error::other)?;
    if url.path().ends_with("/messages") {
        progress.update(|state| state.history_requests += 1);
    }
    let (status, body) = response(&method, &url, total);
    if status != "200 OK" {
        progress.update(|state| state.http_errors += 1);
    }
    let body = body.to_string();
    stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nSet-Cookie: Auth={SESSION_TOKEN}; Path=/\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await?;
    stream.shutdown().await
}

async fn replay(
    socket: &mut WebSocketStream<BufReader<TcpStream>>,
    total: usize,
    count: usize,
    progress: &Progress,
) -> io::Result<()> {
    let subscription = progress
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .connections;
    for index in total.saturating_sub(count)..total {
        let mut message = wire_message(index);
        message["type"] = json!("message");
        message["content"] = json!(format!("synthetic-replay-{subscription}-{index}"));
        socket
            .send(Message::Text(message.to_string().into()))
            .await
            .map_err(io::Error::other)?;
        progress.update(|state| state.emitted_events += 1);
    }
    Ok(())
}

/// Builds synthetic wire rows for matched canonical Store/projection setup.
///
/// # Errors
/// Returns a decoding error if fixture rows no longer match the core model.
pub fn fixture_messages(
    indices: std::ops::Range<usize>,
) -> Result<Vec<papo_core::api::models::Message>, serde_json::Error> {
    indices
        .map(wire_message)
        .map(serde_json::from_value)
        .collect()
}

fn wire_message(index: usize) -> Value {
    json!({
        "id": format!("m{index:08}"), "channel_id": CHANNEL_ID,
        "author_id": format!("fixture-author-{}", index % 8),
        "content": format!("synthetic-{index} {}", "x".repeat(32 + index % 256)),
        "created_at": AT, "attachments": [], "reactions": [],
        "reply_to": (index > 0 && index.is_multiple_of(5)).then(|| format!("m{:08}", index - 1)),
    })
}

fn response(method: &str, url: &url::Url, total: usize) -> (&'static str, Value) {
    let user = json!({"id":ACCOUNT_ID,"username":"fixture-user","roles":[]});
    let body = match (method, url.path()) {
        ("GET", "/auth/whoami") => {
            let mut user = user;
            user["settings"] = json!({"version":1,"config":{}});
            user
        }
        ("POST", "/auth/login" | "/auth/register") => json!({"user":user}),
        ("POST", "/auth/refresh" | "/auth/logout") => json!({}),
        ("GET", "/server") => json!({"id":"fixture-server","name":"fixture","owner_id":ACCOUNT_ID}),
        ("GET", "/channels") => {
            json!({"channels":[{"id":CHANNEL_ID,"name":"fixture","type":"text"}]})
        }
        ("GET", "/dms") => json!({"dms":[]}),
        ("GET", "/users/blocks") => json!({"users":[]}),
        ("GET", "/users") => json!({"users":[user],"has_more":false}),
        ("POST", "/users/profile_batch") => json!({"profiles":[]}),
        ("GET", "/roles") => json!({"roles":[]}),
        ("GET", "/emojis") => json!({"emojis":[],"has_more":false}),
        ("GET", path) if path.ends_with("/notifications") => {
            json!({"notifications":[],"has_more":false})
        }
        ("GET", path) if path == format!("/channels/{CHANNEL_ID}/pinned") => json!({"pinned":[]}),
        ("GET", path) if path == format!("/channels/{CHANNEL_ID}/messages") => {
            let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
            let ascending = query.get("order").is_some_and(|value| value == "asc");
            let limit = query
                .get("limit")
                .and_then(|value| value.parse().ok())
                .unwrap_or(100)
                .min(100);
            let cursor = query.get("last_id");
            let since = query.get("since");
            let pivot = cursor.and_then(|id| id.strip_prefix('m')?.parse::<usize>().ok());
            if cursor.is_some() != since.is_some()
                || cursor.is_some() != pivot.is_some()
                || since.is_some_and(|value| value != AT)
            {
                return ("400 Bad Request", json!({"title":"invalid fixture cursor"}));
            }
            let start = if ascending {
                pivot.map_or(0, |index| index.saturating_add(1).min(total))
            } else {
                0
            };
            let end = if ascending {
                total
            } else {
                pivot.unwrap_or(total).min(total)
            };
            let has_more = end.saturating_sub(start) > limit;
            let indices: Vec<_> = if ascending {
                (start..end).take(limit).collect()
            } else {
                (start..end).rev().take(limit).collect()
            };
            json!({"channel_id":CHANNEL_ID,"messages":indices.into_iter().map(wire_message).collect::<Vec<_>>(),"has_more":has_more})
        }
        ("PUT", path) if path.ends_with("/read") => json!({}),
        _ => return ("404 Not Found", json!({"title":"unknown fixture route"})),
    };
    ("200 OK", body)
}
