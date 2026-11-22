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
    pub(crate) read_requests: Arc<crate::read_requests::ReadRequests>,
    pub(crate) read_acknowledged_at: String,
    pub(crate) holds: Vec<PendingHold>,
    pub(crate) gateway_accepting: bool,
    pub(crate) rest: VecDeque<RestStep>,
    pub(crate) base_url: String,
    pub(crate) message_body: Option<Bytes>,
    pub(crate) received_upload: Option<crate::message_upload::ReceivedUpload>,
    pub(crate) upload_throttle: Option<crate::message_upload::UploadThrottle>,
    pub(crate) upload_byte_holds: Vec<crate::message_upload::UploadByteHold>,
    pub(crate) outgoing: crate::OutgoingRequestCounts,
    pub(crate) silent_outgoing: bool,
    /// Edits applied so far; each edit's `edited_at` is later than the one before.
    pub(crate) edits: u32,
    /// Edits are answered without `content` and `edited_at`.
    pub(crate) sparse_edit_receipts: bool,
    pub(crate) notification_search: crate::notification_search::NotificationSearch,
    pub(crate) media: crate::encoded_media::Media,
    pub(crate) operations: crate::operation_requests::Operations,
    pub(crate) voice: crate::voice::VoiceState,
    pub(crate) next_client_id: u64,
}

/// A running in-memory backend bound exclusively to IPv4 loopback.
///
/// Dropping it aborts the listener and its owned connections. Prefer [`Self::shutdown`]
/// to await completion and observe task failures. Each start resets all counters and scripts.
pub struct TestServer {
    address: SocketAddr,
    disconnects: broadcast::Sender<()>,
    stop_accept: Mutex<Option<oneshot::Sender<oneshot::Sender<()>>>>,
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
            read_requests: Arc::default(),
            read_acknowledged_at: crate::resources::CREATED_AT.into(),
            holds: Vec::new(),
            gateway_accepting: true,
            rest: config.rest.clone().into(),
            base_url: format!("http://{address}"),
            message_body: None,
            received_upload: None,
            upload_throttle: None,
            upload_byte_holds: Vec::new(),
            outgoing: crate::OutgoingRequestCounts::default(),
            silent_outgoing: false,
            edits: 0,
            sparse_edit_receipts: false,
            notification_search: crate::notification_search::NotificationSearch::default(),
            media: crate::encoded_media::Media::default(),
            operations: crate::operation_requests::Operations::default(),
            voice: crate::voice::VoiceState::default(),
            next_client_id: 1,
        }));
        let (stop, stopped) = oneshot::channel();
        let (stop_accept, accept_stopped) = oneshot::channel();
        let (disconnects, _) = broadcast::channel(1);
        let task = tokio::spawn(serve(
            listener,
            Arc::new(config),
            Arc::clone(&state),
            stopped,
            accept_stopped,
            disconnects.clone(),
        ));
        Ok(Self {
            address,
            disconnects,
            stop_accept: Mutex::new(Some(stop_accept)),
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

    /// Stops accepting new TCP connections while keeping existing gateway sockets open, so new
    /// REST dials fail with OS connection refusal (`NotDispatched`) without advancing session epoch.
    pub async fn stop_accepting(&self) {
        let sender = self.stop_accept.lock().await.take();
        if let Some(sender) = sender {
            let (ack_tx, ack_rx) = oneshot::channel();
            if sender.send(ack_tx).is_ok() {
                let _ = ack_rx.await;
            }
        }
    }

    /// Issues a one-use reset capability for a registered fixture user; never logs the token.
    /// Missing usernames return `None`; no production reset-generation endpoint is exposed.
    pub async fn issue_password_reset(&self, username: &str) -> Option<String> {
        self.state.lock().await.auth.issue_password_reset(username)
    }

    /// Renames a registered fixture user and sets its nickname; `whoami` reports both from
    /// then on. Returns false for an unknown user or a taken username.
    pub async fn rename_user(
        &self,
        username: &str,
        new_username: &str,
        nickname: Option<&str>,
    ) -> bool {
        self.state
            .lock()
            .await
            .auth
            .rename_user(username, new_username, nickname)
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
    mut stop_accept: oneshot::Receiver<oneshot::Sender<()>>,
    disconnects: broadcast::Sender<()>,
) -> Result<(), Error> {
    let mut listener = Some(listener);
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = &mut stopped => break,
            ack = &mut stop_accept, if listener.is_some() => {
                listener = None;
                if let Ok(ack) = ack {
                    let _ = ack.send(());
                }
            }
            accepted = async {
                match listener.as_ref() {
                    Some(sock) => sock.accept().await,
                    None => std::future::pending().await,
                }
            }, if listener.is_some() && connections.len() < MAX_CONNECTIONS => {
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
        let (client_id, snapshot, updates) = {
            let mut locked = gateway_state.lock().await;
            let seq = locked.next_client_id;
            locked.next_client_id = locked.next_client_id.saturating_add(1);
            let client_id = format!("client-{seq}");
            let state_ref = &mut *locked;
            let voice_by_user = state_ref.voice.all_user_voice_channels();
            let (snapshot, updates) = state_ref.presence.join_with_voice(&user_id, &voice_by_user);
            (client_id, snapshot, updates)
        };
        gateway::serve_with_state(
            TokioIo::new(upgraded),
            snapshot,
            updates,
            &config.gateway,
            config.ignore_heartbeats,
            disconnects,
            &user_id,
            &client_id,
            Some(Arc::clone(&gateway_state)),
        )
        .await;
        let mut locked = gateway_state.lock().await;
        let state_ref = &mut *locked;
        state_ref
            .voice
            .client_offline(&state_ref.presence, &user_id, &client_id);
        state_ref.presence.leave(&user_id);
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
