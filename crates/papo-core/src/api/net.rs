//! Ponte entre a interface (síncrona, um quadro por vez) e a rede
//! (assíncrona). A janela envia comandos e lê atualizações sem nunca
//! bloquear um quadro; o runtime tokio vive numa thread própria.

use std::sync::mpsc as sync_mpsc;
use std::sync::{Arc, RwLock};

use tokio::sync::mpsc;

use super::client::{Api, ApiError, Session, Upload};
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

#[derive(Debug, Clone)]
pub enum Command {
    Login { username: String, password: String },
    Register { username: String, password: String },
    /// Senha do servidor, para servidores fechados.
    LoginServer { password: String },
    CreateServer { name: String },
    /// Recarrega servidor, canais e pessoas.
    Refresh,
    LoadMessages {
        channel_id: String,
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
        channel_id: String,
        messages: Vec<Message>,
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
    /// Uma notificação que acabou de chegar pelo WebSocket, enriquecida com
    /// channel_id pelo endpoint REST. Diferente de Notifications, isto é um
    /// evento ao vivo e pode virar notificação nativa na plataforma.
    Notification(Box<Notification>),
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
}

pub struct Net {
    commands: mpsc::UnboundedSender<Command>,
    updates: sync_mpsc::Receiver<Update>,
    event_hook: EventHook,
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
    ) -> Self {
        let (commands_tx, commands_rx) = mpsc::unbounded_channel();
        let (updates_tx, updates_rx) = sync_mpsc::channel();
        let event_hook = EventHook::default();
        let worker_event_hook = event_hook.clone();
        let storage_key = crate::server_key(&base_url);
        let session = Arc::new(Session::default());
        session.set_token(load_secret(
            storage.as_ref(),
            &storage_key,
            Secret::SessionToken,
        ));
        let worker_session = Arc::clone(&session);
        let worker_storage = Arc::clone(&storage);
        let worker_storage_key = storage_key.clone();

        std::thread::Builder::new()
            .name("papo-net".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        log::error!("rede sem runtime: {error}");
                        return;
                    }
                };
                runtime.block_on(worker(
                    base_url,
                    worker_storage_key,
                    worker_storage,
                    worker_session,
                    commands_rx,
                    updates_tx,
                    wake,
                    worker_event_hook,
                ));
            })
            .expect("thread de rede");

        Self {
            commands: commands_tx,
            updates: updates_rx,
            event_hook,
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

    pub fn try_recv(&self) -> Option<Update> {
        self.updates.try_recv().ok()
    }

    /// Esquece sessão e senha deste servidor no backend de persistência
    /// escolhido pelo frontend.
    pub fn forget_credentials(&self) {
        if let Err(error) = self.storage.forget_server(&self.storage_key) {
            log::warn!("não foi possível esquecer credenciais: {error}");
        }
        self.session.set_token(None);
    }
}

/// Envia a atualização e acorda a janela: sem isso ela só apareceria na
/// próxima interação do usuário.
fn publish(tx: &sync_mpsc::Sender<Update>, wake: &Wake, update: Update) {
    // O aviso aparece na tela, mas sem uma linha no log uma falha de login
    // era invisível para quem lê o diário depois.
    match &update {
        Update::Error(message) | Update::AuthFailed(message) => {
            log::warn!("falha de rede ou autenticação: {message}");
        }
        _ => {}
    }
    if tx.send(update).is_ok() {
        wake.wake();
    }
}

async fn worker(
    base_url: String,
    storage_key: String,
    storage: Arc<dyn SecretStore>,
    session: Arc<Session>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    updates: sync_mpsc::Sender<Update>,
    wake: Wake,
    event_hook: EventHook,
) {
    let api = match Api::new(&base_url, Arc::clone(&session)) {
        Ok(api) => api,
        Err(error) => {
            publish(&updates, &wake, Update::Error(error.to_string()));
            return;
        }
    };

    // Quem está logado: o Refresh precisa disso para rebuscar as menções.
    let me: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (status_tx, mut status_rx) = mpsc::unbounded_channel();
    let (mut outbound_tx, outbound_rx) = mpsc::unbounded_channel();
    let mut outbound_rx = Some(outbound_rx);
    let mut socket: Option<tokio::task::JoinHandle<()>> = None;

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
        let verified = me.lock().ok().and_then(|slot| slot.clone()).is_some();
        match (session.is_authenticated() && verified, socket.is_some()) {
            (true, false) => {
                if let Some(receiver) = outbound_rx.take() {
                    socket = Some(start_socket(
                        &api,
                        &session,
                        &events_tx,
                        &status_tx,
                        receiver,
                    ));
                }
            }
            (false, true) => {
                if let Some(handle) = socket.take() {
                    handle.abort();
                }
                // Canal novo para o próximo login: o anterior foi junto com a
                // tarefa abortada.
                let (tx, rx) = mpsc::unbounded_channel();
                outbound_tx = tx;
                outbound_rx = Some(rx);
                publish(&updates, &wake, Update::Connection(Connection::Offline));
            }
            _ => {}
        }

        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
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
                    command,
                )
                .await;
            }
            event = events_rx.recv() => {
                let Some(event) = event else { continue };
                event_hook.emit(&event);
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
                        Err(error) => log::warn!("preview {preview_id} não carregou: {error}"),
                    }
                } else if let Event::Notification { ref id, .. } = event {
                    // O evento unicast é deliberadamente pequeno e não traz
                    // channel_id. A listagem REST já contém a forma completa;
                    // resolve só esta notificação para a UI não precisar
                    // adivinhar pelo cache de mensagens.
                    let user_id = me.lock().ok().and_then(|slot| slot.clone());
                    if let Some(user_id) = user_id {
                        match api.notifications(&user_id).await {
                            Ok(notifications) => {
                                if let Some(notification) =
                                    notifications.into_iter().find(|item| item.id == *id)
                                {
                                    publish(
                                        &updates,
                                        &wake,
                                        Update::Notification(Box::new(notification)),
                                    );
                                }
                            }
                            Err(error) => log::warn!("resolver notificação {id}: {error}"),
                        }
                    }
                    publish(&updates, &wake, Update::Event(Box::new(event)));
                } else {
                    publish(&updates, &wake, Update::Event(Box::new(event)));
                }
            }
            status = status_rx.recv() => {
                let Some(status) = status else { continue };
                publish(&updates, &wake, Update::Connection(status));
            }
            _ = verification.tick() => {
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
                if !session.is_authenticated() {
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
                        session.set_token(None);
                        if let Ok(mut slot) = me.lock() {
                            *slot = None;
                        }
                        remove_secret(storage.as_ref(), &storage_key, Secret::SessionToken);
                        publish(&updates, &wake, Update::Session(None));
                    }
                    // Rede fora do ar não encerra a sessão: tenta de novo no
                    // próximo tique.
                    Err(error) => log::warn!("renovação da sessão falhou: {error}"),
                }
            }
        }
    }

    if let Some(socket) = socket {
        socket.abort();
    }
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
    let mut result = api.whoami().await;

    // Servidor fechado é um portão separado da conta. Se já conhecemos a
    // senha do servidor, abre e repete o whoami sem tocar no token do usuário.
    if matches!(result, Err(ApiError::ServerLocked)) {
        match unlock_with_saved(api, storage_key, storage).await {
            Ok(true) => result = api.whoami().await,
            Ok(false) | Err(_) => {
                publish(updates, wake, Update::ServerLocked);
                return;
            }
        }
    }

    match result {
        Ok(whoami) => {
            let id = whoami.id.clone();
            if let Ok(mut slot) = me.lock() {
                *slot = Some(id.clone());
            }
            publish(updates, wake, Update::Session(Some(Box::new(whoami))));
            bootstrap(api, updates, wake, Some(&id)).await;
        }
        Err(ApiError::Unauthorized) => {
            session.set_token(None);
            if let Ok(mut slot) = me.lock() {
                *slot = None;
            }
            remove_secret(storage, storage_key, Secret::SessionToken);
            publish(updates, wake, Update::Session(None));
        }
        Err(ApiError::ServerLocked) => {
            publish(updates, wake, Update::ServerLocked);
        }
        Err(error) => {
            log::warn!("sessão guardada ainda não pôde ser verificada: {error}");
            publish(updates, wake, Update::Connection(Connection::Offline));
            publish(updates, wake, Update::Error(error.to_string()));
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
    match api.login_server(&password).await {
        Ok(()) => Ok(true),
        Err(error) => {
            // A senha do servidor mudou: esquecer é o certo, ou toda entrada
            // tentaria a senha velha antes de perguntar.
            remove_secret(storage, storage_key, Secret::ServerPassword);
            Err(error)
        }
    }
}

fn start_socket(
    api: &Api,
    session: &Arc<Session>,
    events: &mpsc::UnboundedSender<Event>,
    status: &mpsc::UnboundedSender<Connection>,
    outbound: mpsc::UnboundedReceiver<String>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(ws::run(
        api.clone(),
        Arc::clone(session),
        events.clone(),
        status.clone(),
        outbound,
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
                        bootstrap(api, updates, wake, Some(&id)).await;
                    }
                    Err(error) => publish(updates, wake, Update::AuthFailed(error.to_string())),
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
            Err(error) => publish(updates, wake, Update::AuthFailed(error.to_string())),
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
                Err(error) => publish(updates, wake, Update::AuthFailed(error.to_string())),
            }
        }
        Command::CreateServer { name } => match api.create_server(&name).await {
            Ok(_) => {
                let id = me.lock().ok().and_then(|slot| slot.clone());
                bootstrap(api, updates, wake, id.as_deref()).await
            }
            Err(error) => publish(updates, wake, Update::Error(error.to_string())),
        },
        Command::Refresh => {
            let id = me.lock().ok().and_then(|slot| slot.clone());
            bootstrap(api, updates, wake, id.as_deref()).await
        }
        Command::LoadMessages { channel_id } => match api.messages(&channel_id).await {
            Ok(list) => {
                publish(
                    updates,
                    wake,
                    Update::Messages {
                        channel_id: channel_id.clone(),
                        messages: list.messages,
                    },
                );
                // A listagem comum de mensagens não carrega o estado de pin.
                // Reaplica a fonte persistida no banco logo depois.
                load_pinned(api, updates, wake, channel_id).await;
            }
            Err(error) => report(updates, wake, error),
        },
        // As três mexidas em canal terminam iguais: relista os canais, porque
        // a posição dos outros muda junto, e deixa a lista nova ser a verdade.
        Command::CreateChannel { name, kind, topic } => {
            match api.create_channel(&name, &kind, topic.as_deref()).await {
                Ok(channel) => {
                    let id = channel.id.clone();
                    relist_channels(api, updates, wake).await;
                    publish(updates, wake, Update::ChannelCreated(id));
                }
                Err(error) => report(updates, wake, error),
            }
        }
        Command::UpdateChannel {
            channel_id,
            name,
            topic,
        } => match api.update_channel(&channel_id, &name, topic.as_deref()).await {
            Ok(_) => relist_channels(api, updates, wake).await,
            Err(error) => report(updates, wake, error),
        },
        Command::DeleteChannel { channel_id } => match api.delete_channel(&channel_id).await {
            Ok(()) => relist_channels(api, updates, wake).await,
            Err(error) => report(updates, wake, error),
        },
        Command::Search { text } => match api.search(&text).await {
            Ok(found) => publish(updates, wake, Update::SearchResults(found.results)),
            Err(error) => report(updates, wake, error),
        },
        // Mexer em cargo muda quem pode o quê, e isso aparece na lista de
        // pessoas — por isso as duas listas são relidas juntas.
        Command::LoadRoles => relist_roles(api, updates, wake).await,
        Command::UpdateServer(request) => match api.update_server(&request).await {
            Ok(server) => publish(updates, wake, Update::Server(Some(Box::new(server)))),
            Err(error) => report(updates, wake, error),
        },
        // Perfil e presença mudam o que os outros veem na lista de pessoas,
        // então ela é relida logo depois.
        Command::UpdateProfile(request) => {
            let Some(user_id) = me.lock().ok().and_then(|slot| slot.clone()) else {
                return;
            };
            match api.update_profile(&user_id, &request).await {
                Ok(_) => relist_users(api, updates, wake).await,
                Err(error) => report(updates, wake, error),
            }
        }
        Command::SetStatus { status } => {
            let Some(user_id) = me.lock().ok().and_then(|slot| slot.clone()) else {
                return;
            };
            match api.set_status(&user_id, status.as_deref()).await {
                Ok(_) => relist_users(api, updates, wake).await,
                Err(error) => report(updates, wake, error),
            }
        }
        Command::SetAvatar { blob, format } => {
            let Some(user_id) = me.lock().ok().and_then(|slot| slot.clone()) else {
                return;
            };
            match api.set_avatar(&user_id, &blob, &format).await {
                Ok(_) => relist_users(api, updates, wake).await,
                Err(error) => report(updates, wake, error),
            }
        }
        Command::ChangePassword { password } => {
            let Some(user_id) = me.lock().ok().and_then(|slot| slot.clone()) else {
                return;
            };
            match api.change_password(&user_id, &password).await {
                Ok(_) => publish(updates, wake, Update::Done),
                Err(error) => report(updates, wake, error),
            }
        }
        Command::BanUser { user_id, banned } => match api.ban_user(&user_id, banned).await {
            Ok(_) => relist_users(api, updates, wake).await,
            Err(error) => report(updates, wake, error),
        },
        Command::ResetUser { user_id } => match api.reset_user(&user_id).await {
            Ok(_) => relist_users(api, updates, wake).await,
            Err(error) => report(updates, wake, error),
        },
        Command::MoveChannel {
            channel_id,
            old_position,
            new_position,
        } => match api
            .move_channel(&channel_id, old_position, new_position)
            .await
        {
            Ok(_) => relist_channels(api, updates, wake).await,
            Err(error) => report(updates, wake, error),
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
                Err(error) => report(updates, wake, error),
            }
        }
        Command::LoadDevices => match api.connected_devices().await {
            Ok(devices) => publish(updates, wake, Update::Devices(devices)),
            Err(error) => report(updates, wake, error),
        },
        Command::DropConnection { connection_id } => {
            match api.drop_connection(&connection_id).await {
                Ok(_) => match api.connected_devices().await {
                    Ok(devices) => publish(updates, wake, Update::Devices(devices)),
                    Err(error) => report(updates, wake, error),
                },
                Err(error) => report(updates, wake, error),
            }
        }
        Command::CreateEmoji { name, blob, format } => {
            match api.create_emoji(&name, &blob, &format).await {
                Ok(_) => relist_emojis(api, updates, wake).await,
                Err(error) => report(updates, wake, error),
            }
        }
        Command::DeleteEmoji { emoji_id } => match api.delete_emoji(&emoji_id).await {
            Ok(()) => relist_emojis(api, updates, wake).await,
            Err(error) => report(updates, wake, error),
        },
        Command::LoadAuditLogs => match api.audit_logs().await {
            Ok(logs) => publish(updates, wake, Update::AuditLogs(logs)),
            Err(error) => report(updates, wake, error),
        },
        Command::CreateRole {
            name,
            color,
            permissions,
        } => match api.create_role(&name, color.as_deref(), permissions).await {
            Ok(_) => relist_roles(api, updates, wake).await,
            Err(error) => report(updates, wake, error),
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
            Ok(_) => relist_roles(api, updates, wake).await,
            Err(error) => report(updates, wake, error),
        },
        Command::DeleteRole { role_id } => match api.delete_role(&role_id).await {
            Ok(()) => relist_roles(api, updates, wake).await,
            Err(error) => report(updates, wake, error),
        },
        Command::AssignRole { user_id, role_id } => {
            match api.assign_role(&user_id, &role_id).await {
                Ok(_) => relist_roles(api, updates, wake).await,
                Err(error) => report(updates, wake, error),
            }
        }
        Command::UnassignRole { user_id, role_id } => {
            match api.unassign_role(&user_id, &role_id).await {
                Ok(()) => relist_roles(api, updates, wake).await,
                Err(error) => report(updates, wake, error),
            }
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
                Err(error) => report(updates, wake, error),
            }
        }
        Command::EditMessage {
            message_id,
            content,
        } => match api.edit_message(&message_id, &content).await {
            Ok(message) => publish(updates, wake, Update::Edited(Box::new(message))),
            Err(error) => report(updates, wake, error),
        },
        Command::DeleteMessage { message_id } => {
            match api.delete_message(&message_id).await {
                Ok(()) => publish(updates, wake, Update::Deleted(message_id)),
                Err(error) => report(updates, wake, error),
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
                report(updates, wake, error);
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
                Ok(()) => load_pinned(api, updates, wake, channel_id).await,
                Err(error) => report(updates, wake, error),
            }
        }
        Command::LoadPinned { channel_id } => {
            load_pinned(api, updates, wake, channel_id).await
        }
        Command::MarkNotificationsRead { user_id, ids } => {
            if !ids.is_empty()
                && let Err(error) = api.mark_notifications_read(&user_id, ids).await
            {
                log::warn!("marcar notificações: {error}");
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
            Err(error) => publish(updates, wake, Update::AuthFailed(error.to_string())),
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

async fn load_pinned(
    api: &Api,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    channel_id: String,
) {
    match api.pinned(&channel_id).await {
        Ok(list) => {
            let ids = list
                .pinned
                .into_iter()
                .map(|message| message.id)
                .collect();
            publish(updates, wake, Update::Pinned { channel_id, ids });
        }
        Err(ApiError::NotFound) => {}
        Err(error) => log::warn!("fixadas: {error}"),
    }
}

/// Carga inicial depois de autenticar.
async fn bootstrap(
    api: &Api,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    user_id: Option<&str>,
) {
    match api.server().await {
        Ok(server) => publish(updates, wake, Update::Server(server.map(Box::new))),
        Err(error) => report(updates, wake, error),
    }
    match api.channels().await {
        Ok(channels) => publish(updates, wake, Update::Channels(channels)),
        Err(ApiError::NotFound) => {}
        Err(error) => report(updates, wake, error),
    }
    match api.users().await {
        Ok(users) => {
            let ids = users.iter().map(|user| user.id.clone()).collect();
            publish(updates, wake, Update::Users(users));
            load_profiles(api, updates, wake, ids).await;
        }
        Err(ApiError::NotFound) => {}
        Err(error) => report(updates, wake, error),
    }
    match api.roles().await {
        Ok(roles) => publish(updates, wake, Update::Roles(roles)),
        Err(error) => log::warn!("cargos: {error}"),
    }
    match api.emojis().await {
        Ok(emojis) if !emojis.is_empty() => publish(updates, wake, Update::Emojis(emojis)),
        Ok(_) => {}
        Err(error) => log::warn!("emojis: {error}"),
    }
    if let Some(user_id) = user_id {
        match api.notifications(user_id).await {
            Ok(notifications) => {
                publish(updates, wake, Update::Notifications(notifications))
            }
            Err(error) => log::warn!("notificações: {error}"),
        }
    }
}

/// Busca as fotos de perfil de uma vez só. Uma requisição por pessoa seria
/// uma rajada a cada entrada; o `profile_batch` existe justamente para isso.
async fn load_profiles(
    api: &Api,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
    user_ids: Vec<String>,
) {
    if user_ids.is_empty() {
        return;
    }
    match api.profiles(user_ids).await {
        Ok(profiles) => publish(updates, wake, Update::Profiles(profiles)),
        Err(error) => log::warn!("perfis: {error}"),
    }
}

/// Relista as pessoas. Perfil, presença e banimento mudam essa lista.
async fn relist_users(api: &Api, updates: &sync_mpsc::Sender<Update>, wake: &Wake) {
    match api.users().await {
        Ok(users) => {
            let ids = users.iter().map(|user| user.id.clone()).collect();
            publish(updates, wake, Update::Users(users));
            load_profiles(api, updates, wake, ids).await;
        }
        Err(error) => report(updates, wake, error),
    }
}

async fn relist_emojis(api: &Api, updates: &sync_mpsc::Sender<Update>, wake: &Wake) {
    match api.emojis().await {
        Ok(emojis) => publish(updates, wake, Update::Emojis(emojis)),
        Err(error) => report(updates, wake, error),
    }
}

/// Relista cargos e pessoas: um cargo novo muda a cor e as permissões de
/// quem o tem, e as duas listas precisam concordar.
async fn relist_roles(api: &Api, updates: &sync_mpsc::Sender<Update>, wake: &Wake) {
    match api.roles().await {
        Ok(roles) => publish(updates, wake, Update::Roles(roles)),
        Err(error) => report(updates, wake, error),
    }
    match api.users().await {
        Ok(users) => publish(updates, wake, Update::Users(users)),
        Err(error) => log::warn!("pessoas depois do cargo: {error}"),
    }
}

/// Relista os canais depois de mexer em um deles.
async fn relist_channels(
    api: &Api,
    updates: &sync_mpsc::Sender<Update>,
    wake: &Wake,
) {
    match api.channels().await {
        Ok(channels) => publish(updates, wake, Update::Channels(channels)),
        Err(error) => report(updates, wake, error),
    }
}

fn report(updates: &sync_mpsc::Sender<Update>, wake: &Wake, error: ApiError) {
    let update = match error {
        ApiError::Unauthorized => Update::Session(None),
        other => Update::Error(other.to_string()),
    };
    publish(updates, wake, update);
}

// ---------------------------------------------------------------------------
// Persistência de credenciais
// ---------------------------------------------------------------------------

fn load_secret(storage: &dyn SecretStore, server: &str, secret: Secret) -> Option<String> {
    match storage.load(server, secret) {
        Ok(value) => value,
        Err(error) => {
            log::warn!("não foi possível carregar {}: {error}", secret.key());
            None
        }
    }
}

fn store_secret(storage: &dyn SecretStore, server: &str, secret: Secret, value: &str) {
    if let Err(error) = storage.store(server, secret, value) {
        log::warn!("não foi possível guardar {}: {error}", secret.key());
    }
}

fn remove_secret(storage: &dyn SecretStore, server: &str, secret: Secret) {
    if let Err(error) = storage.remove(server, secret) {
        log::warn!("não foi possível apagar {}: {error}", secret.key());
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
