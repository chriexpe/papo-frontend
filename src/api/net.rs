//! Ponte entre a interface (síncrona, um quadro por vez) e a rede
//! (assíncrona). A janela envia comandos e lê atualizações sem nunca
//! bloquear um quadro; o runtime tokio vive numa thread própria.

use std::sync::mpsc as sync_mpsc;
use std::sync::Arc;

use tokio::sync::mpsc;

use super::client::{Api, ApiError, Session, Upload};
use super::models::{
    Channel, Emoji, Message, Notification, ReactionRequest, Server, UserSummary, Whoami,
};
use super::ws::{self, Connection, Event};

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
    SendMessage {
        channel_id: String,
        content: String,
        reply_to: Option<String>,
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
    Event(Box<Event>),
    Connection(Connection),
    Error(String),
}

pub struct Net {
    commands: mpsc::UnboundedSender<Command>,
    updates: sync_mpsc::Receiver<Update>,
    /// A mídia usa o mesmo cookie para baixar anexos.
    pub session: Arc<Session>,
}

impl Net {
    pub fn spawn(base_url: String, repaint: egui::Context) -> Self {
        let (commands_tx, commands_rx) = mpsc::unbounded_channel();
        let (updates_tx, updates_rx) = sync_mpsc::channel();
        let session = Arc::new(Session::default());
        session.set_token(load_token(&base_url));
        let worker_session = Arc::clone(&session);

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
                    worker_session,
                    commands_rx,
                    updates_tx,
                    repaint,
                ));
            })
            .expect("thread de rede");

        Self {
            commands: commands_tx,
            updates: updates_rx,
            session,
        }
    }

    pub fn send(&self, command: Command) {
        let _ = self.commands.send(command);
    }

    pub fn try_recv(&self) -> Option<Update> {
        self.updates.try_recv().ok()
    }
}

/// Envia a atualização e acorda a janela: sem isso ela só apareceria na
/// próxima interação do usuário.
fn publish(tx: &sync_mpsc::Sender<Update>, repaint: &egui::Context, update: Update) {
    if tx.send(update).is_ok() {
        repaint.request_repaint();
    }
}

async fn worker(
    base_url: String,
    session: Arc<Session>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    updates: sync_mpsc::Sender<Update>,
    repaint: egui::Context,
) {
    let api = match Api::new(&base_url, Arc::clone(&session)) {
        Ok(api) => api,
        Err(error) => {
            publish(&updates, &repaint, Update::Error(error.to_string()));
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

    // Sessão guardada em disco: tenta seguir logado sem pedir senha.
    if session.is_authenticated() {
        match api.whoami().await {
            Ok(whoami) => {
                let id = whoami.id.clone();
                if let Ok(mut slot) = me.lock() {
                    *slot = Some(id.clone());
                }
                publish(&updates, &repaint, Update::Session(Some(Box::new(whoami))));
                bootstrap(&api, &updates, &repaint, Some(&id)).await;
            }
            Err(_) => {
                session.set_token(None);
                store_token(&base_url, None);
                publish(&updates, &repaint, Update::Session(None));
            }
        }
    } else {
        publish(&updates, &repaint, Update::Session(None));
    }
    // O token de sessão vale 24 h e a renovação o gira. Seis horas dá quatro
    // chamadas por dia e sobra folga se a máquina dormir um pouco.
    let mut renewal = tokio::time::interval(std::time::Duration::from_secs(6 * 3600));
    renewal.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    renewal.tick().await; // o primeiro tique sai na hora; a sessão acabou de abrir

    loop {
        // O socket acompanha a sessão: abre quando há cookie válido e fecha
        // quando ele some.
        match (session.is_authenticated(), socket.is_some()) {
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
                publish(&updates, &repaint, Update::Connection(Connection::Offline));
            }
            _ => {}
        }

        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else { break };
                handle(&api, &base_url, &session, &me, &updates, &repaint, &outbound_tx, command).await;
            }
            event = events_rx.recv() => {
                let Some(event) = event else { continue };
                publish(&updates, &repaint, Update::Event(Box::new(event)));
            }
            status = status_rx.recv() => {
                let Some(status) = status else { continue };
                publish(&updates, &repaint, Update::Connection(status));
            }
            _ = renewal.tick() => {
                if !session.is_authenticated() {
                    continue;
                }
                match api.refresh().await {
                    // O cookie novo já entrou no pote; só falta o disco.
                    Ok(()) => store_token(&base_url, session.token()),
                    Err(ApiError::Unauthorized) => {
                        session.set_token(None);
                        store_token(&base_url, None);
                        publish(&updates, &repaint, Update::Session(None));
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

/// Apresenta a senha do servidor já guardada. `Ok(false)` quer dizer que não
/// há senha guardada; `Err` que a guardada não serve mais e foi descartada.
async fn unlock_with_saved(api: &Api, base_url: &str) -> Result<bool, ApiError> {
    let Some(password) = load_server_password(base_url) else {
        return Ok(false);
    };
    match api.login_server(&password).await {
        Ok(()) => Ok(true),
        Err(error) => {
            // A senha do servidor mudou: esquecer é o certo, ou toda entrada
            // tentaria a senha velha antes de perguntar.
            if let Some(path) = server_password_path(base_url) {
                let _ = std::fs::remove_file(path);
            }
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
    session: &Arc<Session>,
    me: &Arc<std::sync::Mutex<Option<String>>>,
    updates: &sync_mpsc::Sender<Update>,
    repaint: &egui::Context,
    outbound: &mpsc::UnboundedSender<String>,
    command: Command,
) {
    match command {
        Command::Login { username, password } => match api.login(&username, &password).await {
            Ok(login) => {
                store_token(base_url, session.token());
                if login.connection_violation {
                    publish(updates, repaint, Update::ConnectionViolation);
                }
                match api.whoami().await {
                    Ok(whoami) => {
                        let id = whoami.id.clone();
                        if let Ok(mut slot) = me.lock() {
                            *slot = Some(id.clone());
                        }
                        publish(updates, repaint, Update::Session(Some(Box::new(whoami))));
                        bootstrap(api, updates, repaint, Some(&id)).await;
                    }
                    Err(error) => publish(updates, repaint, Update::AuthFailed(error.to_string())),
                }
            }
            // Servidor fechado: se a senha dele já é conhecida, o portão
            // abre sozinho e o login segue — pedi-la de novo a cada entrada
            // não protege nada, só incomoda.
            Err(ApiError::ServerLocked) => {
                match unlock_with_saved(api, base_url).await {
                    Ok(true) => {
                        Box::pin(handle(
                            api,
                            base_url,
                            session,
                            me,
                            updates,
                            repaint,
                            outbound,
                            Command::Login { username, password },
                        ))
                        .await;
                    }
                    _ => publish(updates, repaint, Update::ServerLocked),
                }
            }
            Err(error) => publish(updates, repaint, Update::AuthFailed(error.to_string())),
        },
        Command::Register { username, password } => {
            match api.register(&username, &password).await {
                Ok(_) => {
                    // O registro não entrega sessão: entra em seguida.
                    Box::pin(handle(
                        api,
                        base_url,
                        session,
                        me,
                        updates,
                        repaint,
                        outbound,
                        Command::Login { username, password },
                    ))
                    .await;
                }
                Err(ApiError::ServerLocked) => {
                    match unlock_with_saved(api, base_url).await {
                        Ok(true) => {
                            Box::pin(handle(
                                api,
                                base_url,
                                session,
                                me,
                                updates,
                                repaint,
                                outbound,
                                Command::Register { username, password },
                            ))
                            .await;
                        }
                        _ => publish(updates, repaint, Update::ServerLocked),
                    }
                }
                Err(error) => publish(updates, repaint, Update::AuthFailed(error.to_string())),
            }
        }
        Command::CreateServer { name } => match api.create_server(&name).await {
            Ok(_) => {
                let id = me.lock().ok().and_then(|slot| slot.clone());
                bootstrap(api, updates, repaint, id.as_deref()).await
            }
            Err(error) => publish(updates, repaint, Update::Error(error.to_string())),
        },
        Command::Refresh => {
            let id = me.lock().ok().and_then(|slot| slot.clone());
            bootstrap(api, updates, repaint, id.as_deref()).await
        }
        Command::LoadMessages { channel_id } => match api.messages(&channel_id).await {
            Ok(list) => publish(
                updates,
                repaint,
                Update::Messages {
                    channel_id,
                    messages: list.messages,
                },
            ),
            Err(error) => report(updates, repaint, error),
        },
        Command::SendMessage {
            channel_id,
            content,
            reply_to,
            attachments,
        } => {
            match api
                .send_message(&channel_id, &content, reply_to.as_deref(), &attachments)
                .await
            {
                Ok(message) => publish(updates, repaint, Update::Sent(Box::new(message))),
                Err(error) => report(updates, repaint, error),
            }
        }
        Command::EditMessage {
            message_id,
            content,
        } => match api.edit_message(&message_id, &content).await {
            Ok(message) => publish(updates, repaint, Update::Edited(Box::new(message))),
            Err(error) => report(updates, repaint, error),
        },
        Command::DeleteMessage { message_id } => {
            match api.delete_message(&message_id).await {
                Ok(()) => publish(updates, repaint, Update::Deleted(message_id)),
                Err(error) => report(updates, repaint, error),
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
                report(updates, repaint, error);
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
                Ok(()) => load_pinned(api, updates, repaint, channel_id).await,
                Err(error) => report(updates, repaint, error),
            }
        }
        Command::LoadPinned { channel_id } => {
            load_pinned(api, updates, repaint, channel_id).await
        }
        Command::MarkNotificationsRead { user_id, ids } => {
            if !ids.is_empty() {
                if let Err(error) = api.mark_notifications_read(&user_id, ids).await {
                    log::warn!("marcar notificações: {error}");
                }
            }
        }
        Command::Typing { channel_id } => {
            let _ = outbound.send(format!(
                r#"{{"type":"typing","channel_id":"{channel_id}"}}"#
            ));
        }
        Command::LoginServer { password } => match api.login_server(&password).await {
            Ok(()) => {
                store_server_password(base_url, &password);
                publish(updates, repaint, Update::ServerUnlocked);
            }
            Err(error) => publish(updates, repaint, Update::AuthFailed(error.to_string())),
        },
        Command::Logout => {
            let _ = api.logout().await;
            store_token(base_url, None);
            publish(updates, repaint, Update::Session(None));
        }
    }
}

async fn load_pinned(
    api: &Api,
    updates: &sync_mpsc::Sender<Update>,
    repaint: &egui::Context,
    channel_id: String,
) {
    match api.pinned(&channel_id).await {
        Ok(list) => {
            let ids = list
                .pinned
                .into_iter()
                .filter_map(|pinned| {
                    pinned
                        .message_id
                        .or_else(|| pinned.message.map(|message| message.id))
                })
                .collect();
            publish(updates, repaint, Update::Pinned { channel_id, ids });
        }
        Err(ApiError::NotFound) => {}
        Err(error) => log::warn!("fixadas: {error}"),
    }
}

/// Carga inicial depois de autenticar.
async fn bootstrap(
    api: &Api,
    updates: &sync_mpsc::Sender<Update>,
    repaint: &egui::Context,
    user_id: Option<&str>,
) {
    match api.server().await {
        Ok(server) => publish(updates, repaint, Update::Server(server.map(Box::new))),
        Err(error) => report(updates, repaint, error),
    }
    match api.channels().await {
        Ok(channels) => publish(updates, repaint, Update::Channels(channels)),
        Err(ApiError::NotFound) => {}
        Err(error) => report(updates, repaint, error),
    }
    match api.users().await {
        Ok(users) => publish(updates, repaint, Update::Users(users)),
        Err(ApiError::NotFound) => {}
        Err(error) => report(updates, repaint, error),
    }
    match api.emojis().await {
        Ok(emojis) if !emojis.is_empty() => publish(updates, repaint, Update::Emojis(emojis)),
        Ok(_) => {}
        Err(error) => log::warn!("emojis: {error}"),
    }
    if let Some(user_id) = user_id {
        match api.notifications(user_id).await {
            Ok(notifications) => {
                publish(updates, repaint, Update::Notifications(notifications))
            }
            Err(error) => log::warn!("notificações: {error}"),
        }
    }
}

fn report(updates: &sync_mpsc::Sender<Update>, repaint: &egui::Context, error: ApiError) {
    let update = match error {
        ApiError::Unauthorized => Update::Session(None),
        other => Update::Error(other.to_string()),
    };
    publish(updates, repaint, update);
}

// ---------------------------------------------------------------------------
// Sessão em disco
// ---------------------------------------------------------------------------

/// Cada servidor guarda o próprio token. Compartilhar um arquivo só seria
/// pior do que perder a sessão: reusar o token de outro servidor conta como
/// reuso de token e derruba todas as sessões da conta.
fn session_path(base_url: &str) -> Option<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "papo")?;
    let dir = dirs.data_dir().join("sessions");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(format!("{}.token", crate::state::server_key(base_url))))
}

/// Apaga o que um servidor deixou em disco. Chamado ao tirá-lo do trilho:
/// sem isso o token e a senha ficariam para sempre.
pub fn forget(base_url: &str) {
    for path in [session_path(base_url), server_password_path(base_url)]
        .into_iter()
        .flatten()
    {
        let _ = std::fs::remove_file(path);
    }
}

/// A senha do servidor mora ao lado do token, com a mesma permissão.
///
/// O token temporário que ela rende vale meia hora, então guardá-lo não
/// adiantaria: o que evita pedir a senha em toda entrada é guardar a senha e
/// reapresentá-la sozinho quando o servidor cobrar.
fn server_password_path(base_url: &str) -> Option<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "papo")?;
    let dir = dirs.data_dir().join("sessions");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(format!("{}.server", crate::state::server_key(base_url))))
}

fn load_server_password(base_url: &str) -> Option<String> {
    let path = server_password_path(base_url)?;
    let password = std::fs::read_to_string(path).ok()?;
    (!password.is_empty()).then_some(password)
}

fn store_server_password(base_url: &str, password: &str) {
    let Some(path) = server_password_path(base_url) else {
        return;
    };
    if std::fs::write(&path, password).is_ok() {
        restrict(&path);
    }
}

fn load_token(base_url: &str) -> Option<String> {
    let path = session_path(base_url)?;
    let token = std::fs::read_to_string(path).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

fn store_token(base_url: &str, token: Option<String>) {
    let Some(path) = session_path(base_url) else {
        return;
    };
    match token {
        Some(token) => {
            if std::fs::write(&path, token).is_ok() {
                restrict(&path);
            }
        }
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// O arquivo guarda um JWT de sessão: só o dono pode ler.
fn restrict(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}
