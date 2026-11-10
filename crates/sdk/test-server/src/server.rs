//! Owns bounded loopback HTTP serving and task lifetime; leaves auth and gateway semantics to handlers.

use std::{
    collections::VecDeque,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
};

use bytes::Bytes;
use hyper::{server::conn::http1, service::service_fn};
use hyper_util::rt::TokioIo;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Mutex, broadcast, mpsc, oneshot},
    task::{JoinHandle, JoinSet},
};

use crate::{
    Config, Error, RestStep, auth::Auth, gateway, presence::Presence, request,
    resources::Resources, rest_hold::PendingHold,
};

pub(crate) const MAX_BODY_BYTES: usize = 128 * 1024;
const MAX_CONNECTIONS: usize = 64;

pub(crate) struct State {
    pub(crate) auth: Auth,
    pub(crate) presence: Presence,
    pub(crate) resources: Resources,
    pub(crate) history: crate::history::History,
    pub(crate) member_requests: crate::MemberRequestCounts,
    pub(crate) holds: Vec<PendingHold>,
    pub(crate) gateway_accepting: bool,
    rest: VecDeque<RestStep>,
    pub(crate) base_url: String,
    pub(crate) message_body: Option<Bytes>,
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
    pub(crate) state: Arc<Mutex<State>>,
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
        let (resources, history) = crate::history_seed::prepare(&config.history)?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|_| Error::Bind)?;
        let address = listener.local_addr().map_err(|_| Error::Bind)?;
        let state = Arc::new(Mutex::new(State {
            auth: Auth::new(),
            presence: Presence::new(),
            resources,
            history,
            member_requests: crate::MemberRequestCounts::default(),
            holds: Vec::new(),
            gateway_accepting: true,
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

    /// Enables or disables new gateway handshakes; admission starts enabled.
    ///
    /// While disabled, `GET /ws` returns HTTP 503 without upgrading. Existing sockets
    /// and REST remain available. Disable admission before [`Self::disconnect_gateways`]
    /// to make a deterministic disconnected mutation window, then re-enable to reconnect.
    pub async fn set_gateway_accepting(&self, accepting: bool) {
        self.state.lock().await.gateway_accepting = accepting;
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
        request::handle(
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
            &user_id,
        )
        .await;
        gateway_state.lock().await.presence.leave(&user_id);
    }
}

pub(crate) fn take_step(state: &mut State, method: &str, path: &str) -> Option<RestStep> {
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

#[cfg(test)]
mod tests;
