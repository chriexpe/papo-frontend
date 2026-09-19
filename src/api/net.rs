//! Ponte entre a interface (síncrona, um quadro por vez) e a rede
//! (assíncrona). A janela envia comandos e lê atualizações sem nunca
//! bloquear um quadro; o runtime tokio vive numa thread própria.

use std::sync::mpsc as sync_mpsc;
use std::sync::Arc;

use tokio::sync::mpsc;

use super::client::{Api, ApiError, Session};
use super::models::{Channel, Message, Server, UserSummary, Whoami};
use super::ws::{self, Connection, Event};

#[derive(Debug, Clone)]
pub enum Command {
    Login { username: String, password: String },
    Register { username: String, password: String },
    CreateServer { name: String },
    /// Recarrega servidor, canais e pessoas.
    Refresh,
    LoadMessages { channel_id: String },
    SendMessage { channel_id: String, content: String },
    Typing { channel_id: String },
    Logout,
}

#[derive(Debug)]
pub enum Update {
    /// Sessão válida (`Some`) ou encerrada (`None`).
    Session(Option<Box<Whoami>>),
    AuthFailed(String),
    Server(Option<Box<Server>>),
    Channels(Vec<Channel>),
    Users(Vec<UserSummary>),
    Messages {
        channel_id: String,
        messages: Vec<Message>,
    },
    Sent(Box<Message>),
    Event(Box<Event>),
    Connection(Connection),
    Error(String),
}

pub struct Net {
    commands: mpsc::UnboundedSender<Command>,
    updates: sync_mpsc::Receiver<Update>,
}

impl Net {
    pub fn spawn(base_url: String, repaint: egui::Context) -> Self {
        let (commands_tx, commands_rx) = mpsc::unbounded_channel();
        let (updates_tx, updates_rx) = sync_mpsc::channel();

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
                runtime.block_on(worker(base_url, commands_rx, updates_tx, repaint));
            })
            .expect("thread de rede");

        Self {
            commands: commands_tx,
            updates: updates_rx,
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
    mut commands: mpsc::UnboundedReceiver<Command>,
    updates: sync_mpsc::Sender<Update>,
    repaint: egui::Context,
) {
    let session = Arc::new(Session::default());
    session.set_token(load_token());

    let api = match Api::new(&base_url, Arc::clone(&session)) {
        Ok(api) => api,
        Err(error) => {
            publish(&updates, &repaint, Update::Error(error.to_string()));
            return;
        }
    };

    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (status_tx, mut status_rx) = mpsc::unbounded_channel();
    let (mut outbound_tx, outbound_rx) = mpsc::unbounded_channel();
    let mut outbound_rx = Some(outbound_rx);
    let mut socket: Option<tokio::task::JoinHandle<()>> = None;

    // Sessão guardada em disco: tenta seguir logado sem pedir senha.
    if session.is_authenticated() {
        match api.whoami().await {
            Ok(me) => {
                publish(&updates, &repaint, Update::Session(Some(Box::new(me))));
                bootstrap(&api, &updates, &repaint).await;
            }
            Err(_) => {
                session.set_token(None);
                store_token(None);
                publish(&updates, &repaint, Update::Session(None));
            }
        }
    } else {
        publish(&updates, &repaint, Update::Session(None));
    }
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
                handle(&api, &session, &updates, &repaint, &outbound_tx, command).await;
            }
            event = events_rx.recv() => {
                let Some(event) = event else { continue };
                publish(&updates, &repaint, Update::Event(Box::new(event)));
            }
            status = status_rx.recv() => {
                let Some(status) = status else { continue };
                publish(&updates, &repaint, Update::Connection(status));
            }
        }
    }

    if let Some(socket) = socket {
        socket.abort();
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

async fn handle(
    api: &Api,
    session: &Arc<Session>,
    updates: &sync_mpsc::Sender<Update>,
    repaint: &egui::Context,
    outbound: &mpsc::UnboundedSender<String>,
    command: Command,
) {
    match command {
        Command::Login { username, password } => match api.login(&username, &password).await {
            Ok(_) => {
                store_token(session.token());
                match api.whoami().await {
                    Ok(me) => {
                        publish(updates, repaint, Update::Session(Some(Box::new(me))));
                        bootstrap(api, updates, repaint).await;
                    }
                    Err(error) => publish(updates, repaint, Update::AuthFailed(error.to_string())),
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
                        session,
                        updates,
                        repaint,
                        outbound,
                        Command::Login { username, password },
                    ))
                    .await;
                }
                Err(error) => publish(updates, repaint, Update::AuthFailed(error.to_string())),
            }
        }
        Command::CreateServer { name } => match api.create_server(&name).await {
            Ok(_) => bootstrap(api, updates, repaint).await,
            Err(error) => publish(updates, repaint, Update::Error(error.to_string())),
        },
        Command::Refresh => bootstrap(api, updates, repaint).await,
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
        } => match api.send_message(&channel_id, &content, None).await {
            Ok(message) => publish(updates, repaint, Update::Sent(Box::new(message))),
            Err(error) => report(updates, repaint, error),
        },
        Command::Typing { channel_id } => {
            let _ = outbound.send(format!(
                r#"{{"type":"typing","channel_id":"{channel_id}"}}"#
            ));
        }
        Command::Logout => {
            let _ = api.logout().await;
            store_token(None);
            publish(updates, repaint, Update::Session(None));
        }
    }
}

/// Carga inicial depois de autenticar.
async fn bootstrap(api: &Api, updates: &sync_mpsc::Sender<Update>, repaint: &egui::Context) {
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

fn session_path() -> Option<std::path::PathBuf> {
    let dirs = directories::ProjectDirs::from("", "", "papo")?;
    let dir = dirs.data_dir().to_path_buf();
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("session"))
}

fn load_token() -> Option<String> {
    let path = session_path()?;
    let token = std::fs::read_to_string(path).ok()?;
    let token = token.trim();
    (!token.is_empty()).then(|| token.to_owned())
}

fn store_token(token: Option<String>) {
    let Some(path) = session_path() else { return };
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
