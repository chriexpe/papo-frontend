//! Owns bounded loopback HTTP serving and task lifetime; leaves auth and gateway semantics to handlers.

use std::{
    collections::VecDeque,
    convert::Infallible,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{Request, Response, body::Incoming, header, server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Mutex, broadcast, mpsc, oneshot},
    task::{JoinHandle, JoinSet},
};

use crate::{
    Config, Error, RestStep,
    auth::{Auth, cookie_value},
    gateway,
    presence::Presence,
    response::{Reply, problem},
};

const MAX_BODY_BYTES: usize = 128 * 1024;
const MAX_CONNECTIONS: usize = 64;

type HttpResponse = Response<Full<Bytes>>;

pub(crate) struct State {
    pub(crate) auth: Auth,
    presence: Presence,
    rest: VecDeque<RestStep>,
    base_url: String,
    message_body: Option<Bytes>,
}

/// A running in-memory backend bound exclusively to IPv4 loopback.
///
/// Dropping it aborts the listener and its owned connections. Prefer [`Self::shutdown`]
/// to await completion and observe task failures. Each start resets all counters and scripts.
pub struct TestServer {
    address: SocketAddr,
    disconnects: broadcast::Sender<()>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), Error>>>,
    state: Arc<Mutex<State>>,
}

impl TestServer {
    /// Starts a fresh backend on an OS-assigned loopback port.
    ///
    /// # Errors
    /// Returns [`Error::Configuration`] for invalid scenarios or [`Error::Bind`] if binding fails.
    pub async fn start(config: Config) -> Result<Self, Error> {
        Self::on_port(config, 0).await
    }

    /// Starts a fresh backend on the requested loopback port; zero asks the OS to choose.
    ///
    /// # Errors
    /// Returns [`Error::Configuration`] for invalid scenarios or [`Error::Bind`] if binding fails.
    pub async fn on_port(config: Config, port: u16) -> Result<Self, Error> {
        validate(&config)?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|_| Error::Bind)?;
        let address = listener.local_addr().map_err(|_| Error::Bind)?;
        let state = Arc::new(Mutex::new(State {
            auth: Auth::new(),
            presence: Presence::new(),
            rest: config.rest.clone().into(),
            base_url: format!("http://{address}"),
            message_body: None,
        }));
        let (stop, stopped) = oneshot::channel();
        let (disconnects, _) = broadcast::channel(1);
        let task = tokio::spawn(serve(
            listener,
            Arc::new(config),
            Arc::clone(&state),
            stopped,
            disconnects.clone(),
        ));
        Ok(Self {
            address,
            disconnects,
            stop: Some(stop),
            task: Some(task),
            state,
        })
    }

    /// Returns the bound loopback address for the caller's requests; never logs it.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// Returns the runtime-derived REST base URL; never logs it.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// Returns the runtime-derived user gateway URL; never logs it.
    #[must_use]
    pub fn gateway_url(&self) -> String {
        format!("ws://{}/ws", self.address)
    }

    /// Drops currently accepted gateway connections without stopping REST or future reconnects.
    /// Call only after observing the client's Online state; no timer or script delay is needed.
    pub fn disconnect_gateways(&self) {
        let _ = self.disconnects.send(());
    }

    /// Issues a one-use reset capability for a registered fixture user; never logs the token.
    /// Missing usernames return `None`; no production reset-generation endpoint is exposed.
    pub async fn issue_password_reset(&self, username: &str) -> Option<String> {
        self.state.lock().await.auth.issue_password_reset(username)
    }

    /// Takes the latest bounded multipart message body, if one was received.
    ///
    /// The body is held only in memory and is never logged.
    pub async fn take_message_body(&self) -> Option<Vec<u8>> {
        self.state
            .lock()
            .await
            .message_body
            .take()
            .map(|body| body.to_vec())
    }

    /// Stops accepting requests, cancels every connection, and joins the server task.
    ///
    /// # Errors
    /// Returns a typed listener or task failure, without including request data.
    pub async fn shutdown(mut self) -> Result<(), Error> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        match self.task.take() {
            Some(task) => task.await.map_err(|_| Error::Task)?,
            None => Ok(()),
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

fn validate(config: &Config) -> Result<(), Error> {
    if config.min_password_chars == 0
        || config.min_password_chars > config.max_password_chars
        || config.max_username_chars < 3
        || config.max_password_chars > MAX_BODY_BYTES
        || config.rest.iter().any(|step| {
            !step.path.starts_with('/')
                || hyper::Method::from_bytes(step.method.as_bytes()).is_err()
                || step
                    .failure
                    .as_ref()
                    .is_some_and(|failure| !(400..=599).contains(&failure.status))
        })
    {
        return Err(Error::Configuration);
    }
    Ok(())
}

async fn serve(
    listener: TcpListener,
    config: Arc<Config>,
    state: Arc<Mutex<State>>,
    mut stopped: oneshot::Receiver<()>,
    disconnects: broadcast::Sender<()>,
) -> Result<(), Error> {
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut stopped => break,
            accepted = listener.accept(), if connections.len() < MAX_CONNECTIONS => {
                let (stream, _) = accepted.map_err(|_| Error::Listener)?;
                connections.spawn(connection(stream, Arc::clone(&config), Arc::clone(&state), disconnects.subscribe()));
            }
            result = connections.join_next(), if !connections.is_empty() => {
                if result.is_some_and(|result| result.is_err()) { return Err(Error::Task); }
            }
        }
    }
    connections.abort_all();
    while let Some(result) = connections.join_next().await {
        if result.is_err_and(|error| !error.is_cancelled()) {
            return Err(Error::Task);
        }
    }
    Ok(())
}

async fn connection(
    stream: TcpStream,
    config: Arc<Config>,
    state: Arc<Mutex<State>>,
    disconnects: broadcast::Receiver<()>,
) {
    let (upgrades, mut upgrade) = mpsc::channel(1);
    let service_config = Arc::clone(&config);
    let gateway_state = Arc::clone(&state);
    let service = service_fn(move |request| {
        handle(
            request,
            Arc::clone(&service_config),
            Arc::clone(&state),
            upgrades.clone(),
        )
    });
    // Peer disconnects and malformed HTTP are connection-local, not server failures.
    if http1::Builder::new()
        .serve_connection(TokioIo::new(stream), service)
        .with_upgrades()
        .await
        .is_err()
    {
        return;
    }
    if let Ok((pending, user_id)) = upgrade.try_recv()
        && let Ok(upgraded) = pending.await
    {
        let (snapshot, updates) = gateway_state.lock().await.presence.join(&user_id);
        gateway::serve(
            TokioIo::new(upgraded),
            snapshot,
            updates,
            &config.gateway,
            config.ignore_heartbeats,
            disconnects,
        )
        .await;
        gateway_state.lock().await.presence.leave(&user_id);
    }
}

async fn handle(
    mut request: Request<Incoming>,
    config: Arc<Config>,
    state: Arc<Mutex<State>>,
    upgrades: mpsc::Sender<(hyper::upgrade::OnUpgrade, String)>,
) -> Result<HttpResponse, Infallible> {
    let method = request.method().as_str().to_owned();
    let path = request.uri().path().to_owned();
    let cookie = cookie_value(request.headers()).map(str::to_owned);
    let (step, base_url) = {
        let mut state = state.lock().await;
        (
            take_step(&mut state, &method, &path),
            state.base_url.clone(),
        )
    };
    let encode = |reply| encode(reply, &base_url);
    if let Some(step) = step {
        tokio::time::sleep(Duration::from_millis(step.delay_ms)).await;
        if let Some(failure) = step.failure {
            return Ok(encode(problem(failure.status, &failure.slug, &path)));
        }
    }
    if method == "GET" && path == "/ws" {
        let user_id = state
            .lock()
            .await
            .auth
            .user_id(cookie.as_deref())
            .map(str::to_owned);
        let Some(user_id) = user_id else {
            return Ok(encode(problem(401, "unauthorized", &path)));
        };
        if !gateway::origin_allowed(request.headers(), &config.allowed_origins) {
            return Ok(encode(problem(403, "ws-upgrade", &path)));
        }
        let response = gateway::upgrade_response(&request);
        let Ok(response) = response else {
            return Ok(encode(problem(400, "ws-upgrade", &path)));
        };
        let pending = hyper::upgrade::on(&mut request);
        if upgrades.try_send((pending, user_id)).is_err() {
            return Ok(encode(problem(500, "internal", &path)));
        }
        return Ok(response);
    }
    let body = Limited::new(request.into_body(), MAX_BODY_BYTES)
        .collect()
        .await;
    let Ok(body) = body else {
        return Ok(encode(problem(413, "invalid-param", &path)));
    };
    let mut state = state.lock().await;
    let body = body.to_bytes();
    if method == "POST" && path == "/messages" {
        if state.auth.user_id(cookie.as_deref()).is_none() {
            return Ok(encode(problem(401, "unauthorized", &path)));
        }
        state.message_body = Some(body);
        return Ok(encode(Reply::json(201, serde_json::json!({}))));
    }
    let reply = state
        .auth
        .handle(&config, &method, &path, cookie.as_deref(), &body);
    if path == "/auth/register"
        && reply.status == 201
        && let Some(user_id) = reply.body.as_ref().and_then(|body| body["id"].as_str())
    {
        state.presence.user_registered(user_id);
    }
    Ok(encode(reply))
}

fn take_step(state: &mut State, method: &str, path: &str) -> Option<RestStep> {
    if state
        .rest
        .front()
        .is_some_and(|step| step.method == method && step.path == path)
    {
        state.rest.pop_front()
    } else {
        None
    }
}

fn encode(mut reply: Reply, base_url: &str) -> HttpResponse {
    if reply.problem
        && let Some(body) = reply.body.as_mut()
        && let Some(slug) = body["type"].as_str()
    {
        body["type"] = format!("{base_url}{slug}").into();
    }
    let has_body = reply.body.is_some();
    let body = reply
        .body
        .map_or_else(Vec::new, |value| value.to_string().into_bytes());
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = hyper::StatusCode::from_u16(reply.status)
        .unwrap_or(hyper::StatusCode::INTERNAL_SERVER_ERROR);
    if has_body {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            header::HeaderValue::from_static(if reply.problem {
                "application/problem+json"
            } else {
                "application/json"
            }),
        );
    }
    if let Some(cookie) = reply
        .cookie
        .and_then(|value| header::HeaderValue::from_str(&value).ok())
    {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    response
}

#[cfg(test)]
mod tests;
