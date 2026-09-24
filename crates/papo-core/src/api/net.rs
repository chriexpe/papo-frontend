//! Ponte entre a interface (síncrona, um quadro por vez) e a rede
//! (assíncrona). A janela envia comandos e lê atualizações sem nunca
//! bloquear um quadro; o runtime tokio vive numa thread própria.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, RwLock};

use tokio::sync::mpsc;
use tokio::task::JoinSet;

use super::client::{Api, ApiError, SendMessageError, Session, Upload};
use crate::cache::{CachedMessage, CachedOutgoing, ClientDb, OutgoingState};
use super::scheduler::{
    DiagnosticJobState, ReconcileKey, ReconcileKind, ReconcilePriority, ReconcileRequest,
    ReconcileScheduler, StartedReconcile, TaskOwner,
};
use super::models::{
    Channel, Emoji, Message, Notification, ReactionRequest, Server, UserSummary, Whoami,
};
use super::ws::{self, Connection, Event};
use crate::storage::{Secret, SecretStore};

/// Acorda a camada de apresentação quando a rede publica uma atualização.
///
/// O núcleo não sabe se há egui, Flutter, Compose ou nenhuma interface:
/// cada frontend fornece apenas o gesto necessário para acordar seu loop.
#[derive(Clone)]
pub struct Wake(Arc<dyn Fn() + Send + Sync>);

impl Wake {
    pub fn new(callback: impl Fn() + Send + Sync + 'static) -> Self {
        Self(Arc::new(callback))
    }

    pub fn noop() -> Self {
        Self::new(|| {})
    }

    fn wake(&self) {
        (self.0)();
    }
}

/// Emissor clonável de comandos para trabalhos que precisam continuar sem
/// depender do laço da interface (por exemplo uma call em segundo plano).
#[derive(Clone)]
pub struct NetSender(mpsc::UnboundedSender<Command>);

impl NetSender {
    pub fn send(&self, command: Command) {
        let _ = self.0.send(command);
    }

    /// Canal avulso para consumidores sem um `Net` completo, como
    /// diagnósticos/headless frontends. A call continua usando exatamente o
    /// mesmo caminho de sinalização da aplicação real.
    pub fn channel() -> (Self, mpsc::UnboundedReceiver<Command>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self(tx), rx)
    }
}

/// Eventos do socket que precisam alcançar um consumidor em tempo real além
/// da Store. A call usa isto para SDP/ICE: a UI continua recebendo o mesmo
/// evento depois, mas o transporte não espera um quadro ser desenhado.
pub type EventCallback = Arc<dyn Fn(&Event) + Send + Sync>;

#[derive(Clone, Default)]
struct EventHook(Arc<RwLock<Option<EventCallback>>>);

impl EventHook {
    fn set(&self, callback: Option<EventCallback>) {
        if let Ok(mut slot) = self.0.write() {
            *slot = callback;
        }
    }

    fn emit(&self, event: &Event) {
        let callback = self.0.read().ok().and_then(|slot| slot.clone());
        if let Some(callback) = callback {
            callback(event);
        }
    }
}

pub type MessageCallback = Arc<dyn Fn(&Message) + Send + Sync>;
pub type NotificationCallback = Arc<dyn Fn(&Notification) + Send + Sync>;

#[derive(Clone, Default)]
struct MessageHook(Arc<RwLock<Option<MessageCallback>>>);

impl MessageHook {
    fn set(&self, callback: Option<MessageCallback>) {
        if let Ok(mut slot) = self.0.write() {
            *slot = callback;
        }
    }

    fn emit(&self, message: &Message) {
        let callback = self.0.read().ok().and_then(|slot| slot.clone());
        if let Some(callback) = callback {
            callback(message);
        }
    }
}

#[derive(Clone, Default)]
struct NotificationHook(Arc<RwLock<Option<NotificationCallback>>>);

impl NotificationHook {
    fn set(&self, callback: Option<NotificationCallback>) {
        if let Ok(mut slot) = self.0.write() {
            *slot = callback;
        }
    }

    fn emit(&self, notification: &Notification) {
        let callback = self.0.read().ok().and_then(|slot| slot.clone());
        if let Some(callback) = callback {
            callback(notification);
        }
    }
}

#[derive(Clone, Default)]
struct WorkerHooks {
    event: EventHook,
    message: MessageHook,
    notification: NotificationHook,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshTicket {
    pub channel_id: String,
    pub generation: u64,
    pub request_id: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NetworkAvailability {
    #[default]
    Unknown,
    Available,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NetworkHint {
    pub availability: NetworkAvailability,
    pub epoch: u64,
}

/// Transport lifetime selected by the owning client runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMode {
    Interactive,
    /// One saved-session REST reconciliation. This mode never opens a WebSocket.
    BackgroundReconcile,
}

impl TransportMode {
    pub fn opens_websocket(self) -> bool {
        matches!(self, Self::Interactive)
    }
}

/// Terminal result emitted by a bounded background transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundRunResult {
    Completed,
    NoSession,
    PermanentAuthFailure,
    ServerLocked,
    TransientFailure,
    Deadline,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkAction {
    None,
    Probe,
    Park,
    Wake,
    Recycle,
}

#[derive(Debug, Default)]
struct NetworkGate {
    hint: NetworkHint,
}

impl NetworkGate {
    fn apply(&mut self, next: NetworkHint) -> NetworkAction {
        let previous = self.hint;
        if previous == next {
            return NetworkAction::None;
        }

        let action = match (previous.availability, next.availability) {
            (_, NetworkAvailability::Unavailable)
                if previous.availability != NetworkAvailability::Unavailable =>
            {
                NetworkAction::Park
            }
            (NetworkAvailability::Unavailable, NetworkAvailability::Available) => {
                NetworkAction::Wake
            }
            (NetworkAvailability::Available, NetworkAvailability::Available)
                if previous.epoch != next.epoch =>
            {
                NetworkAction::Recycle
            }
            (NetworkAvailability::Unknown, NetworkAvailability::Available) => {
                NetworkAction::Probe
            }
            _ => NetworkAction::None,
        };
        self.hint = next;
        action
    }

    fn explicitly_unavailable(&self) -> bool {
        self.hint.availability == NetworkAvailability::Unavailable
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    Login { username: String, password: String },
    Register { username: String, password: String },
    /// Senha do servidor, para servidores fechados.
    LoginServer { password: String },
    CreateServer { name: String },
    /// Recarrega servidor, canais e pessoas.
    Refresh,
    /// Testa imediatamente a saúde do WebSocket atual ou antecipa a próxima
    /// tentativa caso ele já esteja reconectando.
    ProbeConnection,
    /// Dica genérica da plataforma sobre a existência/troca do caminho de
    /// rede. A plataforma nunca altera freshness/generation diretamente.
    NetworkHint(NetworkHint),
    LoadMessages {
        ticket: RefreshTicket,
    },
    CreateChannel {
        name: String,
        kind: String,
        topic: Option<String>,
    },
    UpdateChannel {
        channel_id: String,
        name: String,
        topic: Option<String>,
    },
    DeleteChannel {
        channel_id: String,
    },
    Search {
        text: String,
    },
    LoadRoles,
    CreateRole {
        name: String,
        color: Option<String>,
        permissions: crate::api::models::RolePermissions,
    },
    UpdateRole {
        role_id: String,
        name: String,
        color: Option<String>,
        permissions: crate::api::models::RolePermissions,
    },
    DeleteRole {
        role_id: String,
    },
    AssignRole {
        user_id: String,
        role_id: String,
    },
    UnassignRole {
        user_id: String,
        role_id: String,
    },
    UpdateServer(Box<crate::api::models::UpdateServerRequest>),
    UpdateProfile(Box<crate::api::models::UpdateUserRequest>),
    SetStatus {
        status: Option<String>,
    },
    SetAvatar {
        blob: String,
        format: String,
    },
    ChangePassword {
        password: String,
    },
    BanUser {
        user_id: String,
        banned: bool,
    },
    ResetUser {
        user_id: String,
    },
    MoveChannel {
        channel_id: String,
        old_position: i32,
        new_position: i32,
    },
    SetChannelNotifications {
        channel_id: String,
        setting: String,
    },
    LoadDevices,
    DropConnection {
        connection_id: String,
    },
    CreateEmoji {
        name: String,
        blob: String,
        format: String,
    },
    DeleteEmoji {
        emoji_id: String,
    },
    LoadAuditLogs,
    /// Envio de texto durável. A UI fornece o owner que já estava verificado
    /// na projeção; o worker recusa se ele divergir da sessão verificada.
    QueueMessage {
        local_id: String,
        owner_user_id: String,
        channel_id: String,
        content: String,
        reply_to: Option<String>,
        notify_reply: bool,
        created_at: i64,
    },
    /// Reenvio deliberado pelo usuário. Pode duplicar uma mensagem cujo
    /// resultado anterior era ambíguo; nunca é disparado automaticamente.
    RetryOutgoing {
        local_id: String,
        owner_user_id: String,
    },
    DismissOutgoing {
        local_id: String,
        owner_user_id: String,
    },
    SendMessage {
        channel_id: String,
        content: String,
        reply_to: Option<String>,
        notify_reply: bool,
        attachments: Vec<Upload>,
    },
    EditMessage {
        message_id: String,
        content: String,
    },
    DeleteMessage {
        message_id: String,
    },
    React {
        channel_id: String,
        message_id: String,
        emoji: ReactionRequest,
        add: bool,
    },
    Pin {
        channel_id: String,
        message_id: String,
        pin: bool,
    },
    LoadPinned {
        channel_id: String,
    },
    MarkNotificationsRead {
        user_id: String,
        ids: Vec<String>,
    },
    Typing {
        channel_id: String,
    },
    /// Entra na call do canal: busca os servidores ICE e só então pede a
    /// entrada pelo socket, porque sem ICE a oferta não teria como sair.
    JoinVoice {
        channel_id: String,
        /// Número desta tentativa de entrada, devolvido nas respostas.
        attempt: u64,
    },
    /// Sinalização crua da call (oferta, resposta, candidato, mudo…), já em
    /// JSON: quem a escreve é a thread da call.
    VoiceSignal(String),
    Logout,
}

#[derive(Debug)]
pub enum Update {
    /// Sessão válida (`Some`) ou encerrada (`None`).
    Session(Option<Box<Whoami>>),
    AuthFailed(String),
    /// O servidor é fechado e ainda não recebeu a senha do servidor.
    ServerLocked,
    /// A senha do servidor passou: dá para entrar ou criar conta.
    ServerUnlocked,
    /// O backend detectou reuso de token e derrubou as outras sessões.
    ConnectionViolation,
    Server(Option<Box<Server>>),
    Channels(Vec<Channel>),
    /// Canal recém-criado: a janela o seleciona assim que a lista chega.
    ChannelCreated(String),
    SearchResults(Vec<crate::api::models::SearchResult>),
    Roles(Vec<crate::api::models::Role>),
    Devices(Vec<crate::api::models::ConnectionInfo>),
    AuditLogs(Vec<crate::api::models::AuditLogEntry>),
    Profiles(Vec<crate::api::models::UserProfile>),
    /// Uma operação deu certo e não devolve nada de útil para a tela.
    Done,
    Users(Vec<UserSummary>),
    Messages {
        ticket: RefreshTicket,
        messages: Vec<Message>,
        /// Snapshot de pins lido junto com o histórico. None preserva o
        /// cache anterior caso apenas a consulta de pins falhe.
        pinned_ids: Option<Vec<String>>,
    },
    /// A carga de histórico falhou; a Store libera a tentativa correspondente.
    MessagesFailed(RefreshTicket),
    /// Projeção durável local, criada/restaurada antes de qualquer POST.
    Outgoing(Box<CachedOutgoing>),
    OutgoingRestored(Vec<CachedOutgoing>),
    OutgoingRemoved(String),
    OutgoingRejected {
        content: String,
        reply_to: Option<String>,
        notify_reply: bool,
        message: String,
    },
    SendConfirmed {
        local_id: String,
        message: Box<Message>,
    },
    Sent(Box<Message>),
    Edited(Box<Message>),
    Deleted(String),
    Emojis(Vec<Emoji>),
    Pinned {
        channel_id: String,
        ids: Vec<String>,
    },
    Notifications(Vec<Notification>),
    Event(Box<Event>),
    /// Os servidores ICE chegaram e o `voice_join` já saiu: dá para montar a
    /// call e esperar o `voice_joined`.
    VoiceReady {
        channel_id: String,
        attempt: u64,
        servers: Vec<crate::api::models::IceServer>,
    },
    /// A entrada na call não saiu do chão. Sem isto a tela ficava para
    /// sempre em "conectando", porque não há call nenhuma para desistir.
    VoiceFailed {
        channel_id: String,
        attempt: u64,
        message: String,
    },
    Connection(Connection),
    Error(String),
    /// Terminal marker used only by the bounded background transport.
    BackgroundFinished(BackgroundRunResult),
}

const RECONCILE_CONCURRENCY: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileJobState {
    Queued,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileJobDiagnostics {
    pub key: String,
    pub priority: String,
    pub generation: u64,
    pub session_epoch: u64,
    pub request_id: Option<u64>,
    pub state: ReconcileJobState,
    pub retry_blocked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerDiagnostics {
    pub queued: usize,
    pub running: usize,
    pub capacity: usize,
    pub jobs: Vec<ReconcileJobDiagnostics>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeDiagnostics {
    pub connection: Connection,
    pub network: NetworkHint,
    pub sync_generation: u64,
    pub session_epoch: u64,
    pub scheduler: SchedulerDiagnostics,
}

impl Default for RuntimeDiagnostics {
    fn default() -> Self {
        Self {
            connection: Connection::Offline,
            network: NetworkHint::default(),
            sync_generation: 0,
            session_epoch: 0,
            scheduler: SchedulerDiagnostics {
                queued: 0,
                running: 0,
                capacity: RECONCILE_CONCURRENCY,
                jobs: Vec::new(),
            },
        }
    }
}

struct ReconcileCompletion {
    run_id: u64,
    success: bool,
    updates: Vec<Update>,
}

pub struct Net {
    commands: mpsc::UnboundedSender<Command>,
    updates: sync_mpsc::Receiver<Update>,
    mode: TransportMode,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    worker_thread: Option<std::thread::JoinHandle<()>>,
    event_hook: EventHook,
    message_hook: MessageHook,
    notification_hook: NotificationHook,
    diagnostics: Arc<RwLock<RuntimeDiagnostics>>,
    /// A mídia usa o mesmo cookie para baixar anexos.
    pub session: Arc<Session>,
    storage: Arc<dyn SecretStore>,
    storage_key: String,
}

impl Net {
    pub fn spawn(
        base_url: String,
        wake: Wake,
        storage: Arc<dyn SecretStore>,
        cache: Arc<ClientDb>,
    ) -> Self {
        Self::spawn_with_mode(
            base_url,
            wake,
            storage,
            cache,
            TransportMode::Interactive,
            std::time::Duration::from_secs(25),
        )
    }

    pub fn spawn_background(
        base_url: String,
        wake: Wake,
        storage: Arc<dyn SecretStore>,
        cache: Arc<ClientDb>,
        deadline: std::time::Duration,
    ) -> Self {
        Self::spawn_with_mode(
            base_url,
            wake,
            storage,
            cache,
            TransportMode::BackgroundReconcile,
            deadline,
        )
    }

    fn spawn_with_mode(
        base_url: String,
        wake: Wake,
        storage: Arc<dyn SecretStore>,
        cache: Arc<ClientDb>,
        mode: TransportMode,
        deadline: std::time::Duration,
    ) -> Self {
        let (commands_tx, commands_rx) = mpsc::unbounded_channel();
        let (updates_tx, updates_rx) = sync_mpsc::channel();
        let event_hook = EventHook::default();
        let message_hook = MessageHook::default();
        let notification_hook = NotificationHook::default();
        let worker_hooks = WorkerHooks {
            event: event_hook.clone(),
            message: message_hook.clone(),
            notification: notification_hook.clone(),
        };
        let storage_key = crate::server_key(&base_url);
        let session = Arc::new(Session::default());
        session.set_token(load_secret(
            storage.as_ref(),
            &storage_key,
            Secret::SessionToken,
        ));
        let worker_session = Arc::clone(&session);
        let worker_storage = Arc::clone(&storage);
        let worker_cache = Arc::clone(&cache);
        let worker_storage_key = storage_key.clone();
        let diagnostics = Arc::new(RwLock::new(RuntimeDiagnostics::default()));
        let worker_diagnostics = Arc::clone(&diagnostics);
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);

        let worker_thread = std::thread::Builder::new()
            .name("papo-net".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        log::error!("runtime {}: rede sem runtime: {error}", worker_storage_key);
                        return;
                    }
                };
                if mode.opens_websocket() {
                    runtime.block_on(worker(
                        base_url,
                        worker_storage_key,
                        worker_storage,
                        worker_session,
                        worker_cache,
                        commands_rx,
                        updates_tx,
                        wake,
                        worker_hooks,
                        worker_diagnostics,
                    ));
                } else {
                    runtime.block_on(background_worker(
                        base_url,
                        worker_storage_key,
                        worker_storage,
                        worker_session,
                        worker_cache,
                        updates_tx,
                        wake,
                        worker_diagnostics,
                        worker_cancel,
                        deadline,
                    ));
                }
            })
            .expect("thread de rede");

        Self {
            commands: commands_tx,
            updates: updates_rx,
            mode,
            cancel,
            worker_thread: Some(worker_thread),
            event_hook,
            message_hook,
            notification_hook,
            diagnostics,
            session,
            storage,
            storage_key,
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn sender(&self) -> NetSender {
        NetSender(self.commands.clone())
    }

    pub fn set_event_callback(&self, callback: Option<EventCallback>) {
        self.event_hook.set(callback);
    }

    pub fn set_message_callback(&self, callback: Option<MessageCallback>) {
        self.message_hook.set(callback);
    }

    pub fn set_notification_callback(&self, callback: Option<NotificationCallback>) {
        self.notification_hook.set(callback);
    }

    pub fn try_recv(&self) -> Option<Update> {
        self.updates.try_recv().ok()
    }

    pub fn diagnostics(&self) -> RuntimeDiagnostics {
        self.diagnostics
            .read()
            .map(|snapshot| snapshot.clone())
            .unwrap_or_default()
    }

    pub fn mode(&self) -> TransportMode {
        self.mode
    }

    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<Update> {
        self.updates.recv_timeout(timeout).ok()
    }

    pub fn cancel_background(&self) {
        if self.mode == TransportMode::BackgroundReconcile {
            self.cancel.store(true, std::sync::atomic::Ordering::Release);
        }
    }

    /// Background ownership is not released until this returns. Interactive
    /// workers retain their historical detached lifetime and stop when their
    /// command channel closes.
    pub fn wait_background_shutdown(&mut self) {
        if self.mode != TransportMode::BackgroundReconcile {
            return;
        }
        self.cancel_background();
        if let Some(thread) = self.worker_thread.take() {
            let _ = thread.join();
        }
    }

    /// Esquece sessão e senha deste servidor no backend de persistência
    /// escolhido pelo frontend.
    pub fn forget_credentials(&self) {
        if let Err(error) = self.storage.forget_server(&self.storage_key) {
            log::warn!(
                "runtime {}: não foi possível esquecer credenciais: {error}",
                self.storage_key
            );
        }
        self.session.set_token(None);
    }
}

impl Drop for Net {
    fn drop(&mut self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Release);
        if self.mode == TransportMode::BackgroundReconcile
            && let Some(thread) = self.worker_thread.take()
        {
            let _ = thread.join();
        }
    }
}

/// Envia a atualização e acorda a janela: sem isso ela só apareceria na
/// próxima interação do usuário.
fn publish(tx: &sync_mpsc::Sender<Update>, wake: &Wake, update: Update) {
    if tx.send(update).is_ok() {
        wake.wake();
    }
}

fn publish_runtime(
    scope: &str,
    tx: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    update: Update,
) {
    match &update {
        Update::Error(message) | Update::AuthFailed(message) => {
            log::warn!("runtime {scope}: falha de rede ou autenticação: {message}");
        }
        _ => {}
    }
    publish(tx, wake, update);
}

const OUTGOING_CHANNEL_TTL: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Default)]
struct OutgoingChannelGate {
    ids: HashSet<String>,
    refreshed_at: Option<std::time::Instant>,
}

impl OutgoingChannelGate {
    fn invalidate(&mut self) {
        self.refreshed_at = None;
    }

    async fn refresh_if_needed(&mut self, api: &Api) -> Result<(), ApiError> {
        if self
            .refreshed_at
            .is_some_and(|at| at.elapsed() < OUTGOING_CHANNEL_TTL)
        {
            return Ok(());
        }
        let channels = api.channels().await?;
        self.ids = channels.into_iter().map(|channel| channel.id).collect();
        self.refreshed_at = Some(std::time::Instant::now());
        Ok(())
    }

    fn contains(&self, channel_id: &str) -> bool {
        self.ids.contains(channel_id)
    }
}

fn short_local_id(local_id: &str) -> &str {
    local_id.get(local_id.len().saturating_sub(12)..).unwrap_or(local_id)
}

fn publish_outgoing(
    scope: &str,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    outgoing: &CachedOutgoing,
) {
    log::info!(
        "outgoing {scope}: {:?} local={}",
        outgoing.state,
        short_local_id(&outgoing.local_id)
    );
    publish(updates, wake, Update::Outgoing(Box::new(outgoing.clone())));
}

fn restore_outgoing(
    cache: &ClientDb,
    scope: &str,
    owner: &str,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    outgoing: &mut Vec<CachedOutgoing>,
) {
    match cache.load_outgoing(scope, owner) {
        Ok(mut restored) => {
            restored.sort_by_key(|item| (item.created_at, item.local_id.clone()));
            *outgoing = restored.clone();
            publish(updates, wake, Update::OutgoingRestored(restored));
        }
        Err(error) => {
            log::warn!("outgoing {scope}: restore falhou: {error}");
            publish_runtime(
                scope,
                updates,
                wake,
                Update::Error(format!("fila de envio indisponível: {error}")),
            );
        }
    }
}

fn outgoing_match(
    outgoing: &[CachedOutgoing],
    owner: &str,
    message: &Message,
) -> Option<usize> {
    const WINDOW_MS: i64 = 120_000;
    let content = message.content.as_deref().unwrap_or_default();
    let created_at = message.created_at.timestamp_millis();
    let mut matches = outgoing
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            item.owner_user_id == owner
                && message.author_id == owner
                && matches!(
                    item.state,
                    OutgoingState::Sending | OutgoingState::UnknownOutcome
                )
                && item.channel_id == message.channel_id
                && item.content == content
                && item.reply_to == message.reply_to
                && item.created_at.abs_diff(created_at) <= WINDOW_MS as u64
        })
        .map(|(index, _)| index);
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

fn reconcile_outgoing_message(
    cache: &ClientDb,
    scope: &str,
    owner: &str,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    outgoing: &mut Vec<CachedOutgoing>,
    message: &Message,
) -> bool {
    let Some(index) = outgoing_match(outgoing, owner, message) else {
        return false;
    };
    let local_id = outgoing[index].local_id.clone();
    match cache.confirm_outgoing(scope, &local_id, CachedMessage::from_api(message)) {
        Ok(()) => {
            log::info!(
                "outgoing {scope}: reconciled local={} server_message={}",
                short_local_id(&local_id),
                message.id
            );
            outgoing.remove(index);
            publish(
                updates,
                wake,
                Update::SendConfirmed {
                    local_id,
                    message: Box::new(message.clone()),
                },
            );
            true
        }
        Err(error) => {
            log::warn!(
                "outgoing {scope}: reconciliação durável falhou local={}: {error}",
                short_local_id(&local_id)
            );
            false
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn drive_outgoing(
    api: &Api,
    cache: &ClientDb,
    scope: &str,
    owner: &str,
    network_unavailable: bool,
    channel_gate: &mut OutgoingChannelGate,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    outgoing: &mut Vec<CachedOutgoing>,
) {
    if network_unavailable {
        return;
    }

    if !outgoing
        .iter()
        .any(|item| item.owner_user_id == owner && item.state.may_auto_send())
    {
        return;
    }

    if let Err(error) = channel_gate.refresh_if_needed(api).await {
        log::debug!(
            "outgoing {scope}: channel validation unavailable; keeping queue parked: {error}"
        );
        return;
    }

    while let Some(index) = outgoing
        .iter()
        .enumerate()
        .filter(|(_, item)| item.owner_user_id == owner && item.state.may_auto_send())
        .min_by_key(|(_, item)| (item.created_at, item.local_id.as_str()))
        .map(|(index, _)| index)
    {
        let local_id = outgoing[index].local_id.clone();
        if !channel_gate.contains(&outgoing[index].channel_id) {
            let reason = "channel no longer exists or is not usable".to_owned();
            if let Err(error) = cache.transition_outgoing(
                scope,
                owner,
                &local_id,
                OutgoingState::FailedPermanent,
                Some(reason.clone()),
            ) {
                log::warn!(
                    "outgoing {scope}: could not persist missing-channel failure local={}: {error}",
                    short_local_id(&local_id)
                );
                break;
            }
            outgoing[index].state = OutgoingState::FailedPermanent;
            outgoing[index].last_error = Some(reason);
            publish_outgoing(scope, updates, wake, &outgoing[index]);
            continue;
        }

        if let Err(error) = cache.transition_outgoing(
            scope,
            owner,
            &local_id,
            OutgoingState::Sending,
            None,
        ) {
            log::warn!("outgoing {scope}: não marcou Sending local={}: {error}", short_local_id(&local_id));
            publish_runtime(
                scope,
                updates,
                wake,
                Update::Error(format!("não foi possível preparar a mensagem para envio: {error}")),
            );
            break;
        }

        outgoing[index].state = OutgoingState::Sending;
        outgoing[index].attempt_count = outgoing[index].attempt_count.saturating_add(1);
        outgoing[index].last_attempt_at = Some(crate::cache::now_millis());
        outgoing[index].last_error = None;
        publish_outgoing(scope, updates, wake, &outgoing[index]);

        let attempt = outgoing[index].clone();
        match api
            .send_queued_message(
                &attempt.channel_id,
                &attempt.content,
                attempt.reply_to.as_deref(),
                attempt.notify_reply,
            )
            .await
        {
            Ok(message) => {
                let cached = CachedMessage::from_api(&message);
                if let Err(error) = cache.confirm_outgoing(scope, &local_id, cached) {
                    log::warn!(
                        "outgoing {scope}: confirmação durável falhou local={}: {error}",
                        short_local_id(&local_id)
                    );
                    let _ = cache.transition_outgoing(
                        scope,
                        owner,
                        &local_id,
                        OutgoingState::UnknownOutcome,
                        Some(format!("servidor confirmou, persistência local falhou: {error}")),
                    );
                }
                log::info!(
                    "outgoing {scope}: confirmed local={} server_message={}",
                    short_local_id(&local_id),
                    message.id
                );
                outgoing.remove(index);
                publish(
                    updates,
                    wake,
                    Update::SendConfirmed {
                        local_id,
                        message: Box::new(message),
                    },
                );
            }
            Err(SendMessageError::SafeToRetry(error)) => {
                let message = error.to_string();
                if let Err(db_error) = cache.transition_outgoing(
                    scope,
                    owner,
                    &local_id,
                    OutgoingState::Queued,
                    Some(message.clone()),
                ) {
                    log::warn!(
                        "outgoing {scope}: falhou ao devolver local={} para Queued: {db_error}",
                        short_local_id(&local_id)
                    );
                    outgoing[index].state = OutgoingState::UnknownOutcome;
                    outgoing[index].last_error = Some(db_error);
                    publish_outgoing(scope, updates, wake, &outgoing[index]);
                    break;
                }
                outgoing[index].state = OutgoingState::Queued;
                outgoing[index].last_error = Some(message);
                publish_outgoing(scope, updates, wake, &outgoing[index]);
                // Não gira em loop contra DNS/servidor fora: o timer ou uma
                // mudança de conectividade tentará novamente.
                break;
            }
            Err(SendMessageError::UnknownOutcome(error)) => {
                let message = error.to_string();
                if let Err(db_error) = cache.transition_outgoing(
                    scope,
                    owner,
                    &local_id,
                    OutgoingState::UnknownOutcome,
                    Some(message.clone()),
                ) {
                    log::warn!(
                        "outgoing {scope}: falhou ao persistir UnknownOutcome local={}: {db_error}",
                        short_local_id(&local_id)
                    );
                }
                outgoing[index].state = OutgoingState::UnknownOutcome;
                outgoing[index].last_error = Some(message);
                publish_outgoing(scope, updates, wake, &outgoing[index]);
                // O item ambíguo não bloqueia mensagens posteriores.
                continue;
            }
            Err(SendMessageError::FailedPermanent(error)) => {
                let message = error.to_string();
                if let Err(db_error) = cache.transition_outgoing(
                    scope,
                    owner,
                    &local_id,
                    OutgoingState::FailedPermanent,
                    Some(message.clone()),
                ) {
                    log::warn!(
                        "outgoing {scope}: falhou ao persistir FailedPermanent local={}: {db_error}",
                        short_local_id(&local_id)
                    );
                }
                outgoing[index].state = OutgoingState::FailedPermanent;
                outgoing[index].last_error = Some(message);
                publish_outgoing(scope, updates, wake, &outgoing[index]);
                continue;
            }
        }
    }
}

fn reset_socket_channels(
    outbound_tx: &mut mpsc::UnboundedSender<String>,
    outbound_rx: &mut Option<mpsc::UnboundedReceiver<String>>,
    probe_tx: &mut mpsc::UnboundedSender<()>,
    probe_rx: &mut Option<mpsc::UnboundedReceiver<()>>,
) {
    let (tx, rx) = mpsc::unbounded_channel();
    *outbound_tx = tx;
    *outbound_rx = Some(rx);
    let (tx, rx) = mpsc::unbounded_channel();
    *probe_tx = tx;
    *probe_rx = Some(rx);
}

fn retire_socket(
    socket: &mut Option<tokio::task::JoinHandle<()>>,
    outbound_tx: &mut mpsc::UnboundedSender<String>,
    outbound_rx: &mut Option<mpsc::UnboundedReceiver<String>>,
    probe_tx: &mut mpsc::UnboundedSender<()>,
    probe_rx: &mut Option<mpsc::UnboundedReceiver<()>>,
) -> bool {
    let Some(handle) = socket.take() else {
        return false;
    };
    handle.abort();
    reset_socket_channels(outbound_tx, outbound_rx, probe_tx, probe_rx);
    true
}

fn advance_generation_for_connection(
    previous: Connection,
    next: Connection,
    generation: &mut u64,
) -> bool {
    if previous == Connection::Online && next != Connection::Online {
        *generation = generation.saturating_add(1);
        true
    } else {
        false
    }
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SavedSessionOutcome {
    Verified,
    Unauthorized,
    ServerLocked,
    Transient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BootstrapOutcome {
    Complete,
    Transient,
    Unauthorized,
}

async fn wait_background_cancel(cancel: Arc<std::sync::atomic::AtomicBool>) {
    while !cancel.load(std::sync::atomic::Ordering::Acquire) {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[allow(clippy::too_many_arguments)]
async fn background_worker(
    base_url: String,
    storage_key: String,
    storage: Arc<dyn SecretStore>,
    session: Arc<Session>,
    cache: Arc<ClientDb>,
    updates: sync_mpsc::Sender<Update>,
    wake: Wake,
    diagnostics: Arc<RwLock<RuntimeDiagnostics>>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
    deadline: std::time::Duration,
) {
    // A caller with no remaining budget must not begin I/O at all. Besides
    // making the hard bound explicit, this avoids racing an immediate network
    // error against a zero-duration Tokio timeout.
    if deadline.is_zero() {
        publish(
            &updates,
            &wake,
            Update::BackgroundFinished(BackgroundRunResult::Deadline),
        );
        return;
    }

    // Deliberately separate from worker(): there is no websocket task, heartbeat,
    // reconnect timer, channel-history scheduler, or media/call path here.
    let api = match Api::new(&base_url, Arc::clone(&session)) {
        Ok(api) => api,
        Err(error) => {
            log::warn!("background {storage_key}: REST client unavailable: {error}");
            publish(&updates, &wake, Update::BackgroundFinished(BackgroundRunResult::TransientFailure));
            return;
        }
    };

    if !session.is_authenticated() {
        log::info!("background {storage_key}: no saved session");
        publish(&updates, &wake, Update::BackgroundFinished(BackgroundRunResult::NoSession));
        return;
    }

    let me: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let work = async {
        let outcome = verify_saved_session_once(
            &api,
            &storage_key,
            storage.as_ref(),
            &session,
            &me,
            &updates,
            &wake,
        )
        .await;
        let owner = match outcome {
            SavedSessionOutcome::Verified => {
                me.lock().ok().and_then(|slot| slot.clone()).unwrap_or_default()
            }
            SavedSessionOutcome::Unauthorized => {
                return BackgroundRunResult::PermanentAuthFailure;
            }
            SavedSessionOutcome::ServerLocked => {
                return BackgroundRunResult::ServerLocked;
            }
            SavedSessionOutcome::Transient => {
                return BackgroundRunResult::TransientFailure;
            }
        };
        if owner.is_empty() {
            return BackgroundRunResult::TransientFailure;
        }

        match bootstrap(
            &api,
            &storage_key,
            &updates,
            &wake,
            Some(&owner),
        )
        .await
        {
            BootstrapOutcome::Complete => {}
            BootstrapOutcome::Unauthorized => {
                session.set_token(None);
                remove_secret(storage.as_ref(), &storage_key, Secret::SessionToken);
                publish(&updates, &wake, Update::Session(None));
                return BackgroundRunResult::PermanentAuthFailure;
            }
            BootstrapOutcome::Transient => {
                return BackgroundRunResult::TransientFailure;
            }
        }

        let mut outgoing = Vec::new();
        restore_outgoing(
            cache.as_ref(),
            &storage_key,
            &owner,
            &updates,
            &wake,
            &mut outgoing,
        );
        let mut channel_gate = OutgoingChannelGate::default();
        drive_outgoing(
            &api,
            cache.as_ref(),
            &storage_key,
            &owner,
            false,
            &mut channel_gate,
            &updates,
            &wake,
            &mut outgoing,
        )
        .await;

        BackgroundRunResult::Completed
    };

    let result = tokio::select! {
        _ = wait_background_cancel(Arc::clone(&cancel)) => BackgroundRunResult::Cancelled,
        timed = tokio::time::timeout(deadline, work) => match timed {
            Ok(result) => result,
            Err(_) => BackgroundRunResult::Deadline,
        },
    };

    if let Ok(mut snapshot) = diagnostics.write() {
        snapshot.connection = Connection::Offline;
    }
    log::info!("background {storage_key}: finished {result:?}");
    publish(&updates, &wake, Update::BackgroundFinished(result));
}

#[allow(clippy::too_many_arguments)]
async fn worker(
    base_url: String,
    storage_key: String,
    storage: Arc<dyn SecretStore>,
    session: Arc<Session>,
    cache: Arc<ClientDb>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    updates: sync_mpsc::Sender<Update>,
    wake: Wake,
    hooks: WorkerHooks,
    diagnostics: Arc<RwLock<RuntimeDiagnostics>>,
) {
    let api = match Api::new(&base_url, Arc::clone(&session)) {
        Ok(api) => api,
        Err(error) => {
            publish_runtime(
                &storage_key,
                &updates,
                &wake,
                Update::Error(error.to_string()),
            );
            return;
        }
    };

    // Quem está logado: o Refresh precisa disso para rebuscar as menções.
    let me: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (status_tx, mut status_rx) = mpsc::unbounded_channel();
    let (mut probe_tx, probe_rx) = mpsc::unbounded_channel();
    let mut probe_rx = Some(probe_rx);
    let (mut outbound_tx, outbound_rx) = mpsc::unbounded_channel();
    let mut outbound_rx = Some(outbound_rx);
    let mut socket: Option<tokio::task::JoinHandle<()>> = None;
    let mut recent_message_channels: HashMap<String, String> = HashMap::new();
    let mut reconcile_scheduler =
        ReconcileScheduler::with_scope(RECONCILE_CONCURRENCY, storage_key.clone());
    let mut reconcile_tasks = JoinSet::new();
    let mut reconcile_aborts: HashMap<u64, tokio::task::AbortHandle> = HashMap::new();
    let mut runtime_generation = 0_u64;
    let mut session_epoch = 0_u64;
    let mut worker_connection = Connection::Offline;
    let mut network_gate = NetworkGate::default();
    let mut outgoing: Vec<CachedOutgoing> = Vec::new();
    let mut outgoing_owner: Option<String> = None;
    let mut outgoing_channels = OutgoingChannelGate::default();
    let mut outgoing_retry = tokio::time::interval(std::time::Duration::from_secs(5));
    outgoing_retry.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    outgoing_retry.tick().await;
    let mut diagnostics_dirty = false;
    update_runtime_diagnostics(
        &diagnostics,
        worker_connection,
        network_gate.hint,
        runtime_generation,
        session_epoch,
        &reconcile_scheduler,
    );

    // Sessão persistida pelo frontend: tenta seguir logado sem pedir senha.
    // Falha de rede não é logout. Só uma resposta de autenticação inválida
    // pode apagar o token; qualquer outra falha deixa a sessão guardada para
    // ser verificada de novo quando o servidor voltar.
    if session.is_authenticated() {
        verify_saved_session(
            &api,
            &storage_key,
            storage.as_ref(),
            &session,
            &me,
            &updates,
            &wake,
        )
        .await;
    } else {
        publish(&updates, &wake, Update::Session(None));
    }

    // Enquanto há token mas ainda não conseguimos confirmá-lo, tenta de novo.
    // Isto cobre abrir o app sem internet, DNS fora, Render dormindo e afins.
    let mut verification =
        tokio::time::interval(std::time::Duration::from_secs(5));
    verification.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    verification.tick().await;

    // O token de sessão vale 24 h e a renovação o gira. Seis horas dá quatro
    // chamadas por dia e sobra folga se a máquina dormir um pouco.
    let mut renewal = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
    renewal.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    renewal.tick().await; // o primeiro tique sai na hora; a sessão acabou de abrir

    loop {
        // O socket acompanha a sessão: abre quando há cookie válido e fecha
        // quando ele some.
        let verified_owner = me.lock().ok().and_then(|slot| slot.clone());
        let verified = verified_owner.is_some();
        if verified_owner != outgoing_owner {
            outgoing.clear();
            outgoing_channels.invalidate();
            outgoing_owner = verified_owner.clone();
            if let Some(owner) = verified_owner.as_deref() {
                restore_outgoing(
                    cache.as_ref(),
                    &storage_key,
                    owner,
                    &updates,
                    &wake,
                    &mut outgoing,
                );
            } else {
                publish(&updates, &wake, Update::OutgoingRestored(Vec::new()));
            }
        }
        match (session.is_authenticated() && verified, socket.is_some()) {
            (true, false) if !network_gate.explicitly_unavailable() => {
                if let (Some(receiver), Some(probes)) = (outbound_rx.take(), probe_rx.take()) {
                    socket = Some(start_socket(
                        &api,
                        &session,
                        &storage_key,
                        &events_tx,
                        &status_tx,
                        receiver,
                        probes,
                    ));
                }
            }
            (false, true) => {
                if let Some(handle) = socket.take() {
                    handle.abort();
                }
                // Canal novo para o próximo login: o anterior foi junto com a
                // tarefa abortada.
                reset_socket_channels(
                    &mut outbound_tx,
                    &mut outbound_rx,
                    &mut probe_tx,
                    &mut probe_rx,
                );
                publish(&updates, &wake, Update::Connection(Connection::Offline));
            }
            _ => {}
        }

        diagnostics_dirty |= spawn_ready_reconciles(
            &storage_key,
            &api,
            &mut reconcile_scheduler,
            &mut reconcile_tasks,
            &mut reconcile_aborts,
        );
        if diagnostics_dirty {
            update_runtime_diagnostics(
                &diagnostics,
                worker_connection,
                network_gate.hint,
                runtime_generation,
                session_epoch,
                &reconcile_scheduler,
            );
            diagnostics_dirty = false;
        }
        let scheduler_deadline = reconcile_scheduler.next_ready_at();

        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };

                if let Command::NetworkHint(hint) = &command {
                    let action = network_gate.apply(*hint);
                    diagnostics_dirty = true;
                    if !matches!(action, NetworkAction::None) {
                        outgoing_channels.invalidate();
                    }
                    match action {
                        NetworkAction::None => {}
                        NetworkAction::Probe => {
                            let _ = probe_tx.send(());
                            log::debug!(
                                "runtime {}: network available; probing websocket",
                                storage_key
                            );
                        }
                        NetworkAction::Park => {
                            let retired = retire_socket(
                                &mut socket,
                                &mut outbound_tx,
                                &mut outbound_rx,
                                &mut probe_tx,
                                &mut probe_rx,
                            );
                            if retired || worker_connection == Connection::Online {
                                let _ = status_tx.send(Connection::Offline);
                            }
                            log::info!(
                                "runtime {}: network unavailable; websocket parked",
                                storage_key
                            );
                        }
                        NetworkAction::Wake => {
                            log::info!(
                                "runtime {}: network available; reconnecting immediately",
                                storage_key
                            );
                        }
                        NetworkAction::Recycle => {
                            let retired = retire_socket(
                                &mut socket,
                                &mut outbound_tx,
                                &mut outbound_rx,
                                &mut probe_tx,
                                &mut probe_rx,
                            );
                            if retired || worker_connection == Connection::Online {
                                let _ = status_tx.send(Connection::Offline);
                            }
                            log::info!(
                                "runtime {}: network path changed; recycling websocket",
                                storage_key
                            );
                        }
                    }
                    if !network_gate.explicitly_unavailable()
                        && let Some(owner) = outgoing_owner.as_deref()
                    {
                        drive_outgoing(
                            &api,
                            cache.as_ref(),
                            &storage_key,
                            owner,
                            false,
                            &mut outgoing_channels,
                            &updates,
                            &wake,
                            &mut outgoing,
                        )
                        .await;
                    }
                    continue;
                }

                if matches!(&command, Command::ProbeConnection) {
                    if !network_gate.explicitly_unavailable() {
                        let _ = probe_tx.send(());
                    }
                    continue;
                }

                if matches!(
                    &command,
                    Command::Login { .. } | Command::Register { .. } | Command::Logout
                ) {
                    session_epoch = session_epoch.saturating_add(1);
                    abort_reconciles(
                        reconcile_scheduler.cancel_all(),
                        &mut reconcile_aborts,
                    );
                    diagnostics_dirty = true;
                }

                match command {
                    Command::QueueMessage {
                        local_id,
                        owner_user_id,
                        channel_id,
                        content,
                        reply_to,
                        notify_reply,
                        created_at,
                    } => {
                        let verified_owner =
                            me.lock().ok().and_then(|slot| slot.clone());
                        if !session.is_authenticated()
                            || verified_owner.as_deref() != Some(owner_user_id.as_str())
                        {
                            publish(
                                &updates,
                                &wake,
                                Update::OutgoingRejected {
                                    content,
                                    reply_to,
                                    notify_reply,
                                    message: "sessão não disponível para enfileirar a mensagem"
                                        .to_owned(),
                                },
                            );
                            continue;
                        }

                        let item = CachedOutgoing {
                            local_id,
                            owner_user_id: owner_user_id.clone(),
                            channel_id,
                            content: content.clone(),
                            reply_to: reply_to.clone(),
                            notify_reply,
                            created_at,
                            state: OutgoingState::Queued,
                            attempt_count: 0,
                            last_attempt_at: None,
                            last_error: None,
                        };
                        match cache.enqueue_outgoing(&storage_key, item.clone()) {
                            Ok(()) => {
                                if outgoing_owner.as_deref() != Some(owner_user_id.as_str()) {
                                    outgoing_owner = Some(owner_user_id.clone());
                                    outgoing.clear();
                                }
                                outgoing.push(item.clone());
                                outgoing.sort_by_key(|row| (row.created_at, row.local_id.clone()));
                                publish_outgoing(&storage_key, &updates, &wake, &item);
                                drive_outgoing(
                                    &api,
                                    cache.as_ref(),
                                    &storage_key,
                                    &owner_user_id,
                                    network_gate.explicitly_unavailable(),
                                    &mut outgoing_channels,
                                    &updates,
                                    &wake,
                                    &mut outgoing,
                                )
                                .await;
                            }
                            Err(error) => {
                                log::warn!(
                                    "outgoing {}: enqueue falhou local={}: {error}",
                                    storage_key,
                                    short_local_id(&item.local_id)
                                );
                                publish(
                                    &updates,
                                    &wake,
                                    Update::OutgoingRejected {
                                        content,
                                        reply_to,
                                        notify_reply,
                                        message: format!(
                                            "não foi possível salvar a mensagem antes do envio: {error}"
                                        ),
                                    },
                                );
                            }
                        }
                    }
                    Command::RetryOutgoing {
                        local_id,
                        owner_user_id,
                    } => {
                        let verified_owner =
                            me.lock().ok().and_then(|slot| slot.clone());
                        if !session.is_authenticated()
                            || verified_owner.as_deref() != Some(owner_user_id.as_str())
                        {
                            publish_runtime(
                                &storage_key,
                                &updates,
                                &wake,
                                Update::Error(
                                    "sessão não disponível para reenviar a mensagem".to_owned(),
                                ),
                            );
                            continue;
                        }
                        let Some(index) = outgoing.iter().position(|item| {
                            item.local_id == local_id
                                && item.owner_user_id == owner_user_id
                                && matches!(
                                    item.state,
                                    OutgoingState::UnknownOutcome
                                        | OutgoingState::FailedPermanent
                                )
                        }) else {
                            continue;
                        };
                        match cache.transition_outgoing(
                            &storage_key,
                            &owner_user_id,
                            &local_id,
                            OutgoingState::Queued,
                            None,
                        ) {
                            Ok(()) => {
                                outgoing[index].state = OutgoingState::Queued;
                                outgoing[index].last_error = None;
                                publish_outgoing(
                                    &storage_key,
                                    &updates,
                                    &wake,
                                    &outgoing[index],
                                );
                                drive_outgoing(
                                    &api,
                                    cache.as_ref(),
                                    &storage_key,
                                    &owner_user_id,
                                    network_gate.explicitly_unavailable(),
                                    &mut outgoing_channels,
                                    &updates,
                                    &wake,
                                    &mut outgoing,
                                )
                                .await;
                            }
                            Err(error) => publish_runtime(
                                &storage_key,
                                &updates,
                                &wake,
                                Update::Error(format!(
                                    "não foi possível preparar o reenvio: {error}"
                                )),
                            ),
                        }
                    }
                    Command::DismissOutgoing {
                        local_id,
                        owner_user_id,
                    } => {
                        let verified_owner =
                            me.lock().ok().and_then(|slot| slot.clone());
                        if verified_owner.as_deref() != Some(owner_user_id.as_str()) {
                            continue;
                        }
                        if let Some(index) = outgoing.iter().position(|item| {
                            item.local_id == local_id
                                && item.owner_user_id == owner_user_id
                                && matches!(
                                    item.state,
                                    OutgoingState::UnknownOutcome
                                        | OutgoingState::FailedPermanent
                                )
                        }) {
                            match cache.remove_outgoing(
                                &storage_key,
                                &owner_user_id,
                                &local_id,
                            ) {
                                Ok(()) => {
                                    outgoing.remove(index);
                                    log::info!(
                                        "outgoing {}: dismissed local={}",
                                        storage_key,
                                        short_local_id(&local_id)
                                    );
                                    publish(
                                        &updates,
                                        &wake,
                                        Update::OutgoingRemoved(local_id),
                                    );
                                }
                                Err(error) => publish_runtime(
                                    &storage_key,
                                    &updates,
                                    &wake,
                                    Update::Error(format!(
                                        "não foi possível descartar a mensagem: {error}"
                                    )),
                                ),
                            }
                        }
                    }
                    Command::Refresh => {
                        let user_id = me.lock().ok().and_then(|slot| slot.clone());
                        let result = reconcile_scheduler.submit(
                            ReconcileRequest {
                                owner: TaskOwner {
                                    generation: runtime_generation,
                                    session_epoch,
                                },
                                priority: ReconcilePriority::ActiveServer,
                                kind: ReconcileKind::ServerMetadata { user_id },
                            },
                            std::time::Instant::now(),
                        );
                        abort_reconciles(result.abort_run_ids, &mut reconcile_aborts);
                        diagnostics_dirty = true;
                    }
                    Command::LoadMessages { ticket } => {
                        if ticket.generation != runtime_generation {
                            log::debug!(
                                "reconcile {}: dropped stale ticket channel={} ticket_generation={} runtime_generation={}",
                                storage_key,
                                ticket.channel_id,
                                ticket.generation,
                                runtime_generation
                            );
                            publish(&updates, &wake, Update::MessagesFailed(ticket));
                            continue;
                        }
                        let result = reconcile_scheduler.submit(
                            ReconcileRequest {
                                owner: TaskOwner {
                                    generation: ticket.generation,
                                    session_epoch,
                                },
                                priority: ReconcilePriority::Visible,
                                kind: ReconcileKind::ChannelHistory { ticket },
                            },
                            std::time::Instant::now(),
                        );
                        abort_reconciles(result.abort_run_ids, &mut reconcile_aborts);
                        diagnostics_dirty = true;
                    }
                    other => {
                        handle(
                            &api,
                            &base_url,
                            &storage_key,
                            storage.as_ref(),
                            &session,
                            &me,
                            &updates,
                            &wake,
                            &outbound_tx,
                            other,
                        )
                        .await;
                    }
                }
            }
            event = events_rx.recv() => {
                let Some(event) = event else { continue };
                hooks.event.emit(&event);

                if let Event::Message(message) = &event {
                    if let Some(owner) = outgoing_owner.as_deref() {
                        let _ = reconcile_outgoing_message(
                            cache.as_ref(),
                            &storage_key,
                            owner,
                            &updates,
                            &wake,
                            &mut outgoing,
                            message,
                        );
                    }
                    recent_message_channels
                        .insert(message.id.clone(), message.channel_id.clone());
                    if recent_message_channels.len() > 256 {
                        recent_message_channels.clear();
                        recent_message_channels
                            .insert(message.id.clone(), message.channel_id.clone());
                    }
                    hooks.message.emit(message);
                }

                if let Event::Notification {
                    id,
                    message_id,
                    author_id,
                    preview,
                } = &event {
                    if let Some(channel_id) = message_id
                        .as_ref()
                        .and_then(|message_id| recent_message_channels.get(message_id))
                        .cloned()
                    {
                        hooks.notification.emit(&Notification {
                            id: id.clone(),
                            message_id: message_id.clone(),
                            channel_id: Some(channel_id),
                            author_id: author_id.clone(),
                            message_content: preview.clone(),
                            read: false,
                            created_at: None,
                        });
                    } else if let Some(user_id) =
                        me.lock().ok().and_then(|slot| slot.clone())
                    {
                        let api = api.clone();
                        let notification_hook = hooks.notification.clone();
                        let id = id.clone();
                        let scope = storage_key.clone();
                        tokio::spawn(async move {
                            match api.notifications(&user_id).await {
                                Ok(notifications) => {
                                    if let Some(notification) =
                                        notifications.into_iter().find(|item| item.id == id)
                                    {
                                        notification_hook.emit(&notification);
                                    }
                                }
                                Err(error) => {
                                    log::warn!("runtime {scope}: resolver notificação {id}: {error}");
                                }
                            }
                        });
                    }
                }

                // new_preview traz só o id porque o crawl termina depois da
                // mensagem. Busca o objeto uma vez aqui, fora da thread da UI,
                // para a Store receber o mesmo formato das mensagens listadas.
                if let Event::NewPreview { message_id, preview_id } = event {
                    match api.link_preview(&preview_id).await {
                        Ok(preview) => publish(
                            &updates,
                            &wake,
                            Update::Event(Box::new(Event::LinkPreviewUpdated {
                                message_id,
                                preview,
                            })),
                        ),
                        Err(error) => log::warn!(
                            "runtime {}: preview {preview_id} não carregou: {error}",
                            storage_key
                        ),
                    }
                } else {
                    publish(&updates, &wake, Update::Event(Box::new(event)));
                }
            }
            status = status_rx.recv() => {
                let Some(status) = status else { continue };
                let previous_connection = worker_connection;
                let previous_generation = runtime_generation;
                if advance_generation_for_connection(
                    worker_connection,
                    status,
                    &mut runtime_generation,
                ) {
                    abort_reconciles(
                        reconcile_scheduler.invalidate_owner(TaskOwner {
                            generation: runtime_generation,
                            session_epoch,
                        }),
                        &mut reconcile_aborts,
                    );
                }
                worker_connection = status;
                diagnostics_dirty = true;
                if previous_connection != worker_connection || previous_generation != runtime_generation {
                    log::info!(
                        "runtime {}: connection {:?} -> {:?}, generation {} -> {}",
                        storage_key,
                        previous_connection,
                        worker_connection,
                        previous_generation,
                        runtime_generation
                    );
                }
                publish(&updates, &wake, Update::Connection(status));
            }
            completion = reconcile_tasks.join_next(), if !reconcile_tasks.is_empty() => {
                match completion {
                    Some(Ok(completion)) => {
                        reconcile_aborts.remove(&completion.run_id);
                        let current = reconcile_scheduler.complete(
                            completion.run_id,
                            completion.success,
                            std::time::Instant::now(),
                        );
                        diagnostics_dirty = true;
                        if current {
                            let session_invalid = completion
                                .updates
                                .iter()
                                .any(|update| matches!(update, Update::Session(None)));
                            if session_invalid {
                                session_epoch = session_epoch.saturating_add(1);
                                abort_reconciles(
                                    reconcile_scheduler.cancel_all(),
                                    &mut reconcile_aborts,
                                );
                            }
                            for update in completion.updates {
                                if let Update::Messages { messages, .. } = &update
                                    && let Some(owner) = outgoing_owner.as_deref()
                                {
                                    for message in messages {
                                        let _ = reconcile_outgoing_message(
                                            cache.as_ref(),
                                            &storage_key,
                                            owner,
                                            &updates,
                                            &wake,
                                            &mut outgoing,
                                            message,
                                        );
                                    }
                                }
                                publish_runtime(&storage_key, &updates, &wake, update);
                            }
                        }
                    }
                    Some(Err(error)) if !error.is_cancelled() => {
                        log::warn!(
                            "runtime {}: tarefa de reconciliação falhou: {error}",
                            storage_key
                        );
                    }
                    Some(Err(_)) | None => {}
                }
            }
            _ = async {
                match scheduler_deadline {
                    Some(deadline) => {
                        tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)).await;
                    }
                    None => std::future::pending::<()>().await,
                }
            } => {}

            _ = outgoing_retry.tick() => {
                if let Some(owner) = outgoing_owner.as_deref() {
                    drive_outgoing(
                        &api,
                        cache.as_ref(),
                        &storage_key,
                        owner,
                        network_gate.explicitly_unavailable(),
                        &mut outgoing_channels,
                        &updates,
                        &wake,
                        &mut outgoing,
                    )
                    .await;
                }
            }
            _ = verification.tick() => {
                if network_gate.explicitly_unavailable() {
                    continue;
                }
                let verified = me.lock().ok().and_then(|slot| slot.clone()).is_some();
                if session.is_authenticated() && !verified {
                    verify_saved_session(
                        &api,
                        &storage_key,
                        storage.as_ref(),
                        &session,
                        &me,
                        &updates,
                        &wake,
                    )
                    .await;
                }
            }
            _ = renewal.tick() => {
                if !session.is_authenticated() || network_gate.explicitly_unavailable() {
                    continue;
                }
                match api.refresh().await {
                    // O cookie novo já entrou no pote; falta persistir o token girado.
                    Ok(()) => store_optional_secret(
                        storage.as_ref(),
                        &storage_key,
                        Secret::SessionToken,
                        session.token(),
                    ),
                    Err(ApiError::Unauthorized) => {
                        session_epoch = session_epoch.saturating_add(1);
                        abort_reconciles(
                            reconcile_scheduler.cancel_all(),
                            &mut reconcile_aborts,
                        );
                        diagnostics_dirty = true;
                        session.set_token(None);
                        if let Ok(mut slot) = me.lock() {
                            *slot = None;
                        }
                        remove_secret(storage.as_ref(), &storage_key, Secret::SessionToken);
                        publish(&updates, &wake, Update::Session(None));
                    }
                    // Rede fora do ar não encerra a sessão: tenta de novo no
                    // próximo tique.
                    Err(error) => log::warn!(
                        "runtime {}: renovação da sessão falhou: {error}",
                        storage_key
                    ),
                }
            }
        }
    }

    abort_reconciles(reconcile_scheduler.cancel_all(), &mut reconcile_aborts);
    worker_connection = Connection::Offline;
    update_runtime_diagnostics(
        &diagnostics,
        worker_connection,
        network_gate.hint,
        runtime_generation,
        session_epoch,
        &reconcile_scheduler,
    );
    reconcile_tasks.abort_all();
    while reconcile_tasks.join_next().await.is_some() {}

    if let Some(socket) = socket {
        socket.abort();
    }
}

fn update_runtime_diagnostics(
    shared: &Arc<RwLock<RuntimeDiagnostics>>,
    connection: Connection,
    network: NetworkHint,
    sync_generation: u64,
    session_epoch: u64,
    scheduler: &ReconcileScheduler,
) {
    let scheduler = scheduler.diagnostics(std::time::Instant::now());
    let jobs = scheduler
        .jobs
        .into_iter()
        .map(|job| ReconcileJobDiagnostics {
            key: match job.key {
                ReconcileKey::ChannelHistory(channel_id) => format!("channel:{channel_id}"),
                ReconcileKey::ServerMetadata => "server-metadata".to_owned(),
            },
            priority: match job.priority {
                ReconcilePriority::Background => "background",
                ReconcilePriority::ActiveServer => "active-server",
                ReconcilePriority::UserRequested => "user-requested",
                ReconcilePriority::Visible => "visible",
            }
            .to_owned(),
            generation: job.owner.generation,
            session_epoch: job.owner.session_epoch,
            request_id: job.request_id,
            state: match job.state {
                DiagnosticJobState::Queued => ReconcileJobState::Queued,
                DiagnosticJobState::Running => ReconcileJobState::Running,
            },
            retry_blocked: job.retry_blocked,
        })
        .collect();

    if let Ok(mut snapshot) = shared.write() {
        *snapshot = RuntimeDiagnostics {
            connection,
            network,
            sync_generation,
            session_epoch,
            scheduler: SchedulerDiagnostics {
                queued: scheduler.queued,
                running: scheduler.running,
                capacity: scheduler.capacity,
                jobs,
            },
        };
    }
}

fn abort_reconciles(
    run_ids: Vec<u64>,
    aborts: &mut HashMap<u64, tokio::task::AbortHandle>,
) {
    for run_id in run_ids {
        if let Some(handle) = aborts.remove(&run_id) {
            handle.abort();
        }
    }
}

fn spawn_ready_reconciles(
    scope: &str,
    api: &Api,
    scheduler: &mut ReconcileScheduler,
    tasks: &mut JoinSet<ReconcileCompletion>,
    aborts: &mut HashMap<u64, tokio::task::AbortHandle>,
) -> bool {
    let mut started_any = false;
    for StartedReconcile { run_id, request } in scheduler.start_ready(std::time::Instant::now()) {
        started_any = true;
        let api = api.clone();
        let scope = scope.to_owned();
        let handle =
            tasks.spawn(async move { run_reconcile(api, scope, run_id, request).await });
        aborts.insert(run_id, handle);
    }
    started_any
}

async fn run_reconcile(
    api: Api,
    scope: String,
    run_id: u64,
    request: ReconcileRequest,
) -> ReconcileCompletion {
    match request.kind {
        ReconcileKind::ChannelHistory { ticket } => {
            let channel_id = ticket.channel_id.clone();
            match api.messages(&channel_id).await {
                Ok(list) => ReconcileCompletion {
                    run_id,
                    success: true,
                    updates: vec![Update::Messages {
                        ticket,
                        messages: list.messages,
                        pinned_ids: fetch_pinned_ids(&api, &scope, &channel_id).await,
                    }],
                },
                Err(error) => ReconcileCompletion {
                    run_id,
                    success: false,
                    updates: vec![
                        Update::MessagesFailed(ticket),
                        update_for_error(error),
                    ],
                },
            }
        }
        ReconcileKind::ServerMetadata { user_id } => ReconcileCompletion {
            run_id,
            success: true,
            updates: bootstrap_updates(&api, &scope, user_id.as_deref()).await,
        },
    }
}

fn update_for_error(error: ApiError) -> Update {
    match error {
        ApiError::Unauthorized => Update::Session(None),
        other => Update::Error(other.to_string()),
    }
}

async fn bootstrap_updates(api: &Api, scope: &str, user_id: Option<&str>) -> Vec<Update> {
    let mut updates = Vec::new();

    match api.server().await {
        Ok(server) => updates.push(Update::Server(server.map(Box::new))),
        Err(error) => updates.push(update_for_error(error)),
    }
    match api.channels().await {
        Ok(channels) => updates.push(Update::Channels(channels)),
        Err(ApiError::NotFound) => {}
        Err(error) => updates.push(update_for_error(error)),
    }
    match api.users().await {
        Ok(users) => {
            let ids: Vec<String> = users.iter().map(|user| user.id.clone()).collect();
            updates.push(Update::Users(users));
            if !ids.is_empty() {
                match api.profiles(ids).await {
                    Ok(profiles) => updates.push(Update::Profiles(profiles)),
                    Err(error) => log::warn!("runtime {scope}: perfis: {error}"),
                }
            }
        }
        Err(ApiError::NotFound) => {}
        Err(error) => updates.push(update_for_error(error)),
    }
    match api.roles().await {
        Ok(roles) => updates.push(Update::Roles(roles)),
        Err(error) => log::warn!("runtime {scope}: cargos: {error}"),
    }
    match api.emojis().await {
        Ok(emojis) if !emojis.is_empty() => updates.push(Update::Emojis(emojis)),
        Ok(_) => {}
        Err(error) => log::warn!("runtime {scope}: emojis: {error}"),
    }
    if let Some(user_id) = user_id {
        match api.notifications(user_id).await {
            Ok(notifications) => updates.push(Update::Notifications(notifications)),
            Err(error) => log::warn!("runtime {scope}: notificações: {error}"),
        }
    }

    updates
}

/// Confirma uma sessão persistida sem transformar indisponibilidade em logout.
///
/// Só `Unauthorized` prova que o token deixou de valer. Timeout, DNS, 5xx,
/// resposta incompleta e servidor temporariamente fora mantêm a credencial e
/// deixam o próximo tique tentar novamente.
async fn verify_saved_session(
    api: &Api,
    storage_key: &str,
    storage: &dyn SecretStore,
    session: &Arc<Session>,
    me: &Arc<std::sync::Mutex<Option<String>>>,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
) {
    if verify_saved_session_once(
        api,
        storage_key,
        storage,
        session,
        me,
        updates,
        wake,
    )
    .await
        == SavedSessionOutcome::Verified
    {
        let id = me.lock().ok().and_then(|slot| slot.clone());
        let _ = bootstrap(api, storage_key, updates, wake, id.as_deref()).await;
    }
}

async fn verify_saved_session_once(
    api: &Api,
    storage_key: &str,
    storage: &dyn SecretStore,
    session: &Arc<Session>,
    me: &Arc<std::sync::Mutex<Option<String>>>,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
) -> SavedSessionOutcome {
    let mut result = api.whoami().await;

    // Servidor fechado é um portão separado da conta. Se já conhecemos a
    // senha do servidor, abre e repete o whoami sem tocar no token do usuário.
    if matches!(result, Err(ApiError::ServerLocked)) {
        match unlock_with_saved(api, storage_key, storage).await {
            Ok(true) => result = api.whoami().await,
            Ok(false) => {
                publish(updates, wake, Update::ServerLocked);
                return SavedSessionOutcome::ServerLocked;
            }
            Err(ApiError::Unauthorized) => {
                // Only a definitive rejection invalidates the saved server
                // password. A network/server failure must preserve it.
                remove_secret(storage, storage_key, Secret::ServerPassword);
                publish(updates, wake, Update::ServerLocked);
                return SavedSessionOutcome::ServerLocked;
            }
            Err(error) => {
                log::warn!(
                    "runtime {storage_key}: senha guardada do servidor não pôde ser verificada: {error}"
                );
                publish(updates, wake, Update::Connection(Connection::Offline));
                publish_runtime(
                    storage_key,
                    updates,
                    wake,
                    Update::Error(error.to_string()),
                );
                return SavedSessionOutcome::Transient;
            }
        }
    }

    match result {
        Ok(whoami) => {
            if let Ok(mut slot) = me.lock() {
                *slot = Some(whoami.id.clone());
            }
            publish(updates, wake, Update::Session(Some(Box::new(whoami))));
            SavedSessionOutcome::Verified
        }
        Err(ApiError::Unauthorized) => {
            session.set_token(None);
            if let Ok(mut slot) = me.lock() {
                *slot = None;
            }
            remove_secret(storage, storage_key, Secret::SessionToken);
            publish(updates, wake, Update::Session(None));
            SavedSessionOutcome::Unauthorized
        }
        Err(ApiError::ServerLocked) => {
            publish(updates, wake, Update::ServerLocked);
            SavedSessionOutcome::ServerLocked
        }
        Err(error) => {
            log::warn!(
                "runtime {storage_key}: sessão guardada ainda não pôde ser verificada: {error}"
            );
            publish(updates, wake, Update::Connection(Connection::Offline));
            publish_runtime(
                storage_key,
                updates,
                wake,
                Update::Error(error.to_string()),
            );
            SavedSessionOutcome::Transient
        }
    }
}

/// Apresenta a senha do servidor já guardada. `Ok(false)` quer dizer que não
/// há senha guardada; `Err` que a guardada não serve mais e foi descartada.
async fn unlock_with_saved(
    api: &Api,
    storage_key: &str,
    storage: &dyn SecretStore,
) -> Result<bool, ApiError> {
    let Some(password) = load_secret(storage, storage_key, Secret::ServerPassword) else {
        return Ok(false);
    };
    api.login_server(&password).await.map(|()| true)
}

fn start_socket(
    api: &Api,
    session: &Arc<Session>,
    scope: &str,
    events: &mpsc::UnboundedSender<Event>,
    status: &mpsc::UnboundedSender<Connection>,
    outbound: mpsc::UnboundedReceiver<String>,
    probe: mpsc::UnboundedReceiver<()>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(ws::run(
        api.clone(),
        Arc::clone(session),
        scope.to_owned(),
        events.clone(),
        status.clone(),
        outbound,
        probe,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn handle(
    api: &Api,
    base_url: &str,
    storage_key: &str,
    storage: &dyn SecretStore,
    session: &Arc<Session>,
    me: &Arc<std::sync::Mutex<Option<String>>>,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    outbound: &mpsc::UnboundedSender<String>,
    command: Command,
) {
    match command {
        Command::Login { username, password } => match api.login(&username, &password).await {
            Ok(login) => {
                store_optional_secret(
                    storage,
                    storage_key,
                    Secret::SessionToken,
                    session.token(),
                );
                if login.connection_violation {
                    publish(updates, wake, Update::ConnectionViolation);
                }
                match api.whoami().await {
                    Ok(whoami) => {
                        let id = whoami.id.clone();
                        if let Ok(mut slot) = me.lock() {
                            *slot = Some(id.clone());
                        }
                        publish(updates, wake, Update::Session(Some(Box::new(whoami))));
                        bootstrap(api, storage_key, updates, wake, Some(&id)).await;
                    }
                    Err(error) => publish_runtime(
                storage_key,
                updates,
                wake,
                Update::AuthFailed(error.to_string()),
            ),
                }
            }
            // Servidor fechado: se a senha dele já é conhecida, o portão
            // abre sozinho e o login segue — pedi-la de novo a cada entrada
            // não protege nada, só incomoda.
            Err(ApiError::ServerLocked) => {
                match unlock_with_saved(api, storage_key, storage).await {
                    Ok(true) => {
                        Box::pin(handle(
                            api,
                            base_url,
                            storage_key,
                            storage,
                            session,
                            me,
                            updates,
                            wake,
                            outbound,
                            Command::Login { username, password },
                        ))
                        .await;
                    }
                    _ => publish(updates, wake, Update::ServerLocked),
                }
            }
            Err(error) => publish_runtime(
                storage_key,
                updates,
                wake,
                Update::AuthFailed(error.to_string()),
            ),
        },
        Command::Register { username, password } => {
            match api.register(&username, &password).await {
                Ok(_) => {
                    // O registro não entrega sessão: entra em seguida.
                    Box::pin(handle(
                        api,
                        base_url,
                        storage_key,
                        storage,
                        session,
                        me,
                        updates,
                        wake,
                        outbound,
                        Command::Login { username, password },
                    ))
                    .await;
                }
                Err(ApiError::ServerLocked) => {
                    match unlock_with_saved(api, storage_key, storage).await {
                        Ok(true) => {
                            Box::pin(handle(
                                api,
                                base_url,
                                storage_key,
                                storage,
                                session,
                                me,
                                updates,
                                wake,
                                outbound,
                                Command::Register { username, password },
                            ))
                            .await;
                        }
                        _ => publish(updates, wake, Update::ServerLocked),
                    }
                }
                Err(error) => publish_runtime(
                storage_key,
                updates,
                wake,
                Update::AuthFailed(error.to_string()),
            ),
            }
        }
        Command::CreateServer { name } => match api.create_server(&name).await {
            Ok(_) => {
                let id = me.lock().ok().and_then(|slot| slot.clone());
                let _ = bootstrap(api, storage_key, updates, wake, id.as_deref()).await;
            }
            Err(error) => publish_runtime(
                storage_key,
                updates,
                wake,
                Update::Error(error.to_string()),
            ),
        },
        // Reconciliação é consumida no laço do worker e executada pelo
        // scheduler; nunca deve entrar no caminho ordenado abaixo.
        Command::Refresh
        | Command::LoadMessages { .. }
        | Command::ProbeConnection
        | Command::NetworkHint(_) => {}
        // As três mexidas em canal terminam iguais: relista os canais, porque
        // a posição dos outros muda junto, e deixa a lista nova ser a verdade.
        Command::CreateChannel { name, kind, topic } => {
            match api.create_channel(&name, &kind, topic.as_deref()).await {
                Ok(channel) => {
                    let id = channel.id.clone();
                    relist_channels(api, storage_key, updates, wake).await;
                    publish(updates, wake, Update::ChannelCreated(id));
                }
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::UpdateChannel {
            channel_id,
            name,
            topic,
        } => match api.update_channel(&channel_id, &name, topic.as_deref()).await {
            Ok(_) => relist_channels(api, storage_key, updates, wake).await,
            Err(error) => report(storage_key, updates, wake, error),
        },
        Command::DeleteChannel { channel_id } => match api.delete_channel(&channel_id).await {
            Ok(()) => relist_channels(api, storage_key, updates, wake).await,
            Err(error) => report(storage_key, updates, wake, error),
        },
        Command::Search { text } => match api.search(&text).await {
            Ok(found) => publish(updates, wake, Update::SearchResults(found.results)),
            Err(error) => report(storage_key, updates, wake, error),
        },
        // Mexer em cargo muda quem pode o quê, e isso aparece na lista de
        // pessoas — por isso as duas listas são relidas juntas.
        Command::LoadRoles => relist_roles(api, storage_key, updates, wake).await,
        Command::UpdateServer(request) => match api.update_server(&request).await {
            Ok(server) => publish(updates, wake, Update::Server(Some(Box::new(server)))),
            Err(error) => report(storage_key, updates, wake, error),
        },
        // Perfil e presença mudam o que os outros veem na lista de pessoas,
        // então ela é relida logo depois.
        Command::UpdateProfile(request) => {
            let Some(user_id) = me.lock().ok().and_then(|slot| slot.clone()) else {
                return;
            };
            match api.update_profile(&user_id, &request).await {
                Ok(_) => relist_users(api, storage_key, updates, wake).await,
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::SetStatus { status } => {
            let Some(user_id) = me.lock().ok().and_then(|slot| slot.clone()) else {
                return;
            };
            match api.set_status(&user_id, status.as_deref()).await {
                Ok(_) => relist_users(api, storage_key, updates, wake).await,
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::SetAvatar { blob, format } => {
            let Some(user_id) = me.lock().ok().and_then(|slot| slot.clone()) else {
                return;
            };
            match api.set_avatar(&user_id, &blob, &format).await {
                Ok(_) => relist_users(api, storage_key, updates, wake).await,
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::ChangePassword { password } => {
            let Some(user_id) = me.lock().ok().and_then(|slot| slot.clone()) else {
                return;
            };
            match api.change_password(&user_id, &password).await {
                Ok(_) => publish(updates, wake, Update::Done),
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::BanUser { user_id, banned } => match api.ban_user(&user_id, banned).await {
            Ok(_) => relist_users(api, storage_key, updates, wake).await,
            Err(error) => report(storage_key, updates, wake, error),
        },
        Command::ResetUser { user_id } => match api.reset_user(&user_id).await {
            Ok(_) => relist_users(api, storage_key, updates, wake).await,
            Err(error) => report(storage_key, updates, wake, error),
        },
        Command::MoveChannel {
            channel_id,
            old_position,
            new_position,
        } => match api
            .move_channel(&channel_id, old_position, new_position)
            .await
        {
            Ok(_) => relist_channels(api, storage_key, updates, wake).await,
            Err(error) => report(storage_key, updates, wake, error),
        },
        Command::SetChannelNotifications {
            channel_id,
            setting,
        } => {
            let Some(user_id) = me.lock().ok().and_then(|slot| slot.clone()) else {
                return;
            };
            match api
                .set_channel_notifications(&channel_id, &user_id, &setting)
                .await
            {
                Ok(_) => publish(updates, wake, Update::Done),
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::LoadDevices => match api.connected_devices().await {
            Ok(devices) => publish(updates, wake, Update::Devices(devices)),
            Err(error) => report(storage_key, updates, wake, error),
        },
        Command::DropConnection { connection_id } => {
            match api.drop_connection(&connection_id).await {
                Ok(_) => match api.connected_devices().await {
                    Ok(devices) => publish(updates, wake, Update::Devices(devices)),
                    Err(error) => report(storage_key, updates, wake, error),
                },
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::CreateEmoji { name, blob, format } => {
            match api.create_emoji(&name, &blob, &format).await {
                Ok(_) => relist_emojis(api, storage_key, updates, wake).await,
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::DeleteEmoji { emoji_id } => match api.delete_emoji(&emoji_id).await {
            Ok(()) => relist_emojis(api, storage_key, updates, wake).await,
            Err(error) => report(storage_key, updates, wake, error),
        },
        Command::LoadAuditLogs => match api.audit_logs().await {
            Ok(logs) => publish(updates, wake, Update::AuditLogs(logs)),
            Err(error) => report(storage_key, updates, wake, error),
        },
        Command::CreateRole {
            name,
            color,
            permissions,
        } => match api.create_role(&name, color.as_deref(), permissions).await {
            Ok(_) => relist_roles(api, storage_key, updates, wake).await,
            Err(error) => report(storage_key, updates, wake, error),
        },
        Command::UpdateRole {
            role_id,
            name,
            color,
            permissions,
        } => match api
            .update_role(&role_id, &name, color.as_deref(), permissions)
            .await
        {
            Ok(_) => relist_roles(api, storage_key, updates, wake).await,
            Err(error) => report(storage_key, updates, wake, error),
        },
        Command::DeleteRole { role_id } => match api.delete_role(&role_id).await {
            Ok(()) => relist_roles(api, storage_key, updates, wake).await,
            Err(error) => report(storage_key, updates, wake, error),
        },
        Command::AssignRole { user_id, role_id } => {
            match api.assign_role(&user_id, &role_id).await {
                Ok(_) => relist_roles(api, storage_key, updates, wake).await,
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::UnassignRole { user_id, role_id } => {
            match api.unassign_role(&user_id, &role_id).await {
                Ok(()) => relist_roles(api, storage_key, updates, wake).await,
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::QueueMessage { .. }
        | Command::RetryOutgoing { .. }
        | Command::DismissOutgoing { .. } => {
            unreachable!("comando de fila é interceptado pelo worker antes do handler legado")
        }
        Command::SendMessage {
            channel_id,
            content,
            reply_to,
            notify_reply,
            attachments,
        } => {
            match api
                .send_message(
                    &channel_id,
                    &content,
                    reply_to.as_deref(),
                    notify_reply,
                    &attachments,
                )
                .await
            {
                Ok(message) => publish(updates, wake, Update::Sent(Box::new(message))),
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::EditMessage {
            message_id,
            content,
        } => match api.edit_message(&message_id, &content).await {
            Ok(message) => publish(updates, wake, Update::Edited(Box::new(message))),
            Err(error) => report(storage_key, updates, wake, error),
        },
        Command::DeleteMessage { message_id } => {
            match api.delete_message(&message_id).await {
                Ok(()) => publish(updates, wake, Update::Deleted(message_id)),
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::React {
            channel_id,
            message_id,
            emoji,
            add,
        } => {
            let outcome = if add {
                api.add_reaction(&channel_id, &message_id, &emoji)
                    .await
                    .map(|_| ())
            } else {
                api.remove_reaction(&channel_id, &message_id, &emoji).await
            };
            if let Err(error) = outcome {
                report(storage_key, updates, wake, error);
            }
        }
        Command::Pin {
            channel_id,
            message_id,
            pin,
        } => {
            let outcome = if pin {
                api.pin_message(&channel_id, &message_id).await.map(|_| ())
            } else {
                api.unpin_message(&channel_id, &message_id).await
            };
            match outcome {
                Ok(()) => load_pinned(api, storage_key, updates, wake, channel_id).await,
                Err(error) => report(storage_key, updates, wake, error),
            }
        }
        Command::LoadPinned { channel_id } => {
            load_pinned(api, storage_key, updates, wake, channel_id).await
        }
        Command::MarkNotificationsRead { user_id, ids } => {
            if !ids.is_empty()
                && let Err(error) = api.mark_notifications_read(&user_id, ids).await
            {
                log::warn!("runtime {storage_key}: marcar notificações: {error}");
            }
        }
        Command::JoinVoice {
            channel_id,
            attempt,
        } => match api.ice_servers().await {
            Ok(servers) => {
                publish(
                    updates,
                    wake,
                    Update::VoiceReady {
                        channel_id: channel_id.clone(),
                        attempt,
                        servers,
                    },
                );
                let _ = outbound.send(format!(
                    r#"{{"type":"voice_join","channel_id":"{channel_id}"}}"#
                ));
            }
            Err(error) => publish(
                updates,
                wake,
                Update::VoiceFailed {
                    channel_id,
                    attempt,
                    message: error.to_string(),
                },
            ),
        },
        Command::VoiceSignal(json) => {
            let _ = outbound.send(json);
        }
        Command::Typing { channel_id } => {
            let _ = outbound.send(format!(
                r#"{{"type":"typing","channel_id":"{channel_id}"}}"#
            ));
        }
        Command::LoginServer { password } => match api.login_server(&password).await {
            Ok(()) => {
                store_secret(storage, storage_key, Secret::ServerPassword, &password);
                publish(updates, wake, Update::ServerUnlocked);

                // Se chegamos ao portão com uma sessão já persistida (caso
                // típico ao reabrir o app), terminar o unlock deve retomar a
                // sessão sozinho — não obrigar usuário e senha outra vez.
                let verified = me.lock().ok().and_then(|slot| slot.clone()).is_some();
                if session.is_authenticated() && !verified {
                    verify_saved_session(
                        api,
                        storage_key,
                        storage,
                        session,
                        me,
                        updates,
                        wake,
                    )
                    .await;
                }
            }
            Err(error) => publish_runtime(
                storage_key,
                updates,
                wake,
                Update::AuthFailed(error.to_string()),
            ),
        },
        Command::Logout => {
            let _ = api.logout().await;
            session.set_token(None);
            if let Ok(mut slot) = me.lock() {
                *slot = None;
            }
            remove_secret(storage, storage_key, Secret::SessionToken);
            publish(updates, wake, Update::Session(None));
        }
    }
}

async fn fetch_pinned_ids(api: &Api, scope: &str, channel_id: &str) -> Option<Vec<String>> {
    match api.pinned(channel_id).await {
        Ok(list) => Some(
            list.pinned
                .into_iter()
                .map(|message| message.id)
                .collect(),
        ),
        Err(ApiError::NotFound) => Some(Vec::new()),
        Err(error) => {
            log::warn!("runtime {scope}: fixadas: {error}");
            None
        }
    }
}

async fn load_pinned(
    api: &Api,
    storage_key: &str,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    channel_id: String,
) {
    if let Some(ids) = fetch_pinned_ids(api, storage_key, &channel_id).await {
        publish(updates, wake, Update::Pinned { channel_id, ids });
    }
}

/// Carga inicial depois de autenticar.
async fn bootstrap(
    api: &Api,
    storage_key: &str,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    user_id: Option<&str>,
) -> BootstrapOutcome {
    let mut complete = true;
    let mut unauthorized = false;

    match api.server().await {
        Ok(server) => publish(updates, wake, Update::Server(server.map(Box::new))),
        Err(error) => {
            unauthorized |= matches!(error, ApiError::Unauthorized);
            complete = false;
            report(storage_key, updates, wake, error);
        }
    }
    match api.channels().await {
        Ok(channels) => publish(updates, wake, Update::Channels(channels)),
        Err(ApiError::NotFound) => {}
        Err(error) => {
            unauthorized |= matches!(error, ApiError::Unauthorized);
            complete = false;
            report(storage_key, updates, wake, error);
        }
    }
    match api.users().await {
        Ok(users) => {
            let ids = users.iter().map(|user| user.id.clone()).collect();
            publish(updates, wake, Update::Users(users));
            if let Err(error) = load_profiles(api, updates, wake, ids).await {
                unauthorized |= matches!(error, ApiError::Unauthorized);
                complete = false;
                log::warn!("runtime {storage_key}: perfis: {error}");
            }
        }
        Err(ApiError::NotFound) => {}
        Err(error) => {
            unauthorized |= matches!(error, ApiError::Unauthorized);
            complete = false;
            report(storage_key, updates, wake, error);
        }
    }
    match api.roles().await {
        Ok(roles) => publish(updates, wake, Update::Roles(roles)),
        Err(error) => {
            unauthorized |= matches!(error, ApiError::Unauthorized);
            complete = false;
            log::warn!("runtime {storage_key}: cargos: {error}");
        }
    }
    match api.emojis().await {
        Ok(emojis) if !emojis.is_empty() => publish(updates, wake, Update::Emojis(emojis)),
        Ok(_) => {}
        Err(error) => {
            unauthorized |= matches!(error, ApiError::Unauthorized);
            complete = false;
            log::warn!("runtime {storage_key}: emojis: {error}");
        }
    }
    if let Some(user_id) = user_id {
        match api.notifications(user_id).await {
            Ok(notifications) => publish(updates, wake, Update::Notifications(notifications)),
            Err(error) => {
                unauthorized |= matches!(error, ApiError::Unauthorized);
                complete = false;
                log::warn!("runtime {storage_key}: notificações: {error}");
            }
        }
    }

    if unauthorized {
        BootstrapOutcome::Unauthorized
    } else if complete {
        BootstrapOutcome::Complete
    } else {
        BootstrapOutcome::Transient
    }
}

/// Busca as fotos de perfil de uma vez só. Uma requisição por pessoa seria
/// uma rajada a cada entrada; o `profile_batch` existe justamente para isso.
async fn load_profiles(
    api: &Api,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    user_ids: Vec<String>,
) -> Result<(), ApiError> {
    if user_ids.is_empty() {
        return Ok(());
    }
    let profiles = api.profiles(user_ids).await?;
    publish(updates, wake, Update::Profiles(profiles));
    Ok(())
}

/// Relista as pessoas. Perfil, presença e banimento mudam essa lista.
async fn relist_users(
    api: &Api,
    storage_key: &str,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
) {
    match api.users().await {
        Ok(users) => {
            let ids = users.iter().map(|user| user.id.clone()).collect();
            publish(updates, wake, Update::Users(users));
            if let Err(error) = load_profiles(api, updates, wake, ids).await {
                log::warn!("runtime {storage_key}: perfis: {error}");
            }
        }
        Err(error) => report(storage_key, updates, wake, error),
    }
}

async fn relist_emojis(
    api: &Api,
    storage_key: &str,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
) {
    match api.emojis().await {
        Ok(emojis) => publish(updates, wake, Update::Emojis(emojis)),
        Err(error) => report(storage_key, updates, wake, error),
    }
}

/// Relista cargos e pessoas: um cargo novo muda a cor e as permissões de
/// quem o tem, e as duas listas precisam concordar.
async fn relist_roles(
    api: &Api,
    storage_key: &str,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
) {
    match api.roles().await {
        Ok(roles) => publish(updates, wake, Update::Roles(roles)),
        Err(error) => report(storage_key, updates, wake, error),
    }
    match api.users().await {
        Ok(users) => publish(updates, wake, Update::Users(users)),
        Err(error) => log::warn!(
            "runtime {storage_key}: pessoas depois do cargo: {error}"
        ),
    }
}

/// Relista os canais depois de mexer em um deles.
async fn relist_channels(
    api: &Api,
    storage_key: &str,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
) {
    match api.channels().await {
        Ok(channels) => publish(updates, wake, Update::Channels(channels)),
        Err(error) => report(storage_key, updates, wake, error),
    }
}

fn report(
    storage_key: &str,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    error: ApiError,
) {
    let update = match error {
        ApiError::Unauthorized => Update::Session(None),
        other => Update::Error(other.to_string()),
    };
    publish_runtime(storage_key, updates, wake, update);
}

// ---------------------------------------------------------------------------
// Persistência de credenciais
// ---------------------------------------------------------------------------

fn load_secret(storage: &dyn SecretStore, server: &str, secret: Secret) -> Option<String> {
    match storage.load(server, secret) {
        Ok(value) => value,
        Err(error) => {
            log::warn!(
                "runtime {server}: não foi possível carregar {}: {error}",
                secret.key()
            );
            None
        }
    }
}

fn store_secret(storage: &dyn SecretStore, server: &str, secret: Secret, value: &str) {
    if let Err(error) = storage.store(server, secret, value) {
        log::warn!(
            "runtime {server}: não foi possível guardar {}: {error}",
            secret.key()
        );
    }
}

fn remove_secret(storage: &dyn SecretStore, server: &str, secret: Secret) {
    if let Err(error) = storage.remove(server, secret) {
        log::warn!(
            "runtime {server}: não foi possível apagar {}: {error}",
            secret.key()
        );
    }
}

fn store_optional_secret(
    storage: &dyn SecretStore,
    server: &str,
    secret: Secret,
    value: Option<String>,
) {
    match value {
        Some(value) => store_secret(storage, server, secret, &value),
        None => remove_secret(storage, server, secret),
    }
}


#[cfg(test)]
mod network_hint_tests {
    use super::*;

    fn hint(availability: NetworkAvailability, epoch: u64) -> NetworkHint {
        NetworkHint { availability, epoch }
    }

    #[test]
    fn unknown_available_only_probes() {
        let mut gate = NetworkGate::default();
        assert_eq!(
            gate.apply(hint(NetworkAvailability::Available, 1)),
            NetworkAction::Probe
        );
        assert_eq!(
            gate.apply(hint(NetworkAvailability::Available, 1)),
            NetworkAction::None
        );
    }

    #[test]
    fn unavailable_is_parked_once_and_restore_wakes() {
        let mut gate = NetworkGate::default();
        assert_eq!(
            gate.apply(hint(NetworkAvailability::Unavailable, 1)),
            NetworkAction::Park
        );
        assert_eq!(
            gate.apply(hint(NetworkAvailability::Unavailable, 1)),
            NetworkAction::None
        );
        assert_eq!(
            gate.apply(hint(NetworkAvailability::Available, 2)),
            NetworkAction::Wake
        );
    }

    #[test]
    fn path_change_recycles_once_and_same_path_is_ignored() {
        let mut gate = NetworkGate::default();
        assert_eq!(
            gate.apply(hint(NetworkAvailability::Available, 4)),
            NetworkAction::Probe
        );
        assert_eq!(
            gate.apply(hint(NetworkAvailability::Available, 5)),
            NetworkAction::Recycle
        );
        assert_eq!(
            gate.apply(hint(NetworkAvailability::Available, 5)),
            NetworkAction::None
        );
    }

    #[test]
    fn continuity_break_advances_generation_only_once() {
        let mut generation = 12;
        assert!(advance_generation_for_connection(
            Connection::Online,
            Connection::Offline,
            &mut generation,
        ));
        assert_eq!(generation, 13);
        assert!(!advance_generation_for_connection(
            Connection::Offline,
            Connection::Offline,
            &mut generation,
        ));
        assert_eq!(generation, 13);
    }

    #[test]
    fn healthy_probe_transition_does_not_advance_generation() {
        let mut generation = 7;
        assert!(!advance_generation_for_connection(
            Connection::Online,
            Connection::Online,
            &mut generation,
        ));
        assert_eq!(generation, 7);
    }

    fn outgoing_row(id: &str, content: &str, state: OutgoingState) -> CachedOutgoing {
        CachedOutgoing {
            local_id: id.to_owned(),
            owner_user_id: "me".to_owned(),
            channel_id: "general".to_owned(),
            content: content.to_owned(),
            reply_to: None,
            notify_reply: false,
            created_at: crate::cache::now_millis(),
            state,
            attempt_count: 1,
            last_attempt_at: None,
            last_error: None,
        }
    }

    fn server_message(content: &str) -> Message {
        Message {
            id: "server-message".to_owned(),
            channel_id: "general".to_owned(),
            author_id: "me".to_owned(),
            content: Some(content.to_owned()),
            created_at: chrono::Utc::now(),
            edited_at: None,
            reply_to: None,
            attachments: Vec::new(),
            previews: Vec::new(),
            reactions: Vec::new(),
            user_reactions: Vec::new(),
        }
    }

    #[test]
    fn strong_unique_outgoing_match_can_reconcile() {
        let rows = vec![outgoing_row(
            "local-a",
            "hello",
            OutgoingState::UnknownOutcome,
        )];
        assert_eq!(outgoing_match(&rows, "me", &server_message("hello")), Some(0));
    }

    #[test]
    fn identical_outgoing_candidates_are_left_ambiguous() {
        let rows = vec![
            outgoing_row("local-a", "hello", OutgoingState::UnknownOutcome),
            outgoing_row("local-b", "hello", OutgoingState::Sending),
        ];
        assert_eq!(outgoing_match(&rows, "me", &server_message("hello")), None);
    }

    #[test]
    fn queued_item_is_never_consumed_by_server_reconciliation() {
        let rows = vec![outgoing_row("local-a", "hello", OutgoingState::Queued)];
        assert_eq!(outgoing_match(&rows, "me", &server_message("hello")), None);
    }

    #[test]
    fn content_alone_is_not_enough_for_outgoing_match() {
        let mut row = outgoing_row("local-a", "hello", OutgoingState::UnknownOutcome);
        row.channel_id = "other".to_owned();
        assert_eq!(
            outgoing_match(&[row], "me", &server_message("hello")),
            None
        );
    }

    #[test]
    fn another_authors_identical_message_never_resolves_outgoing() {
        let row = outgoing_row("local-a", "hello", OutgoingState::UnknownOutcome);
        let mut message = server_message("hello");
        message.author_id = "someone-else".to_owned();
        assert_eq!(outgoing_match(&[row], "me", &message), None);
    }
}
