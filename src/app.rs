//! Aplicação: junta estado, tema e telas.

use egui::Color32;

use crate::i18n::Lang;
use crate::media::Media;
use crate::platform::files::{self, Chosen, Dialogs, ImagePick};
use crate::platform::menu::{MenuCommand, MenuModel, MenuNode};
use crate::platform::desktop;
#[cfg(target_os = "linux")]
use crate::platform::activate::Activator;
#[cfg(target_os = "linux")]
use crate::platform::notify::{Notification, Notifier};
#[cfg(target_os = "linux")]
use crate::platform::tray::{Tray, TrayCommand, TrayLabels};
#[cfg(target_os = "linux")]
use crate::platform::launcher::{Badge, Launcher};
#[cfg(target_os = "linux")]
use crate::platform::{appmenu::AppMenuSurface, blur::BlurSurface, global_menu::GlobalMenu};
use crate::api::net::{Command, Net, Wake};
use crate::state::{Phase, Screen, Store};
use papo_core::cache::ClientDb;
use papo_core::storage::{Secret, SecretStore};
use crate::voice::{Call, IceConfig};
use crate::ui::auth::{self, AuthAction, AuthForm};

/// Endereço interno usado enquanto o cartão de "adicionar servidor" ainda é
/// só um rascunho. Ele nunca é persistido nem mostrado ao usuário e, por ser
/// um domínio reservado, não pode colidir com um servidor real.
const DRAFT_SERVER_URL: &str = "https://add-server.invalid";
use crate::ui::shell::{self, ChatAction, UiState};
use crate::ui::glass::GlassRenderer;
use crate::ui::theme::{self, Appearance, ThemePref, Tokens};

/// Para onde vai um anexo salvo.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DownloadMode {
    /// Direto para uma pasta, sem diálogo.
    Folder(std::path::PathBuf),
    /// O diálogo do sistema pergunta a cada anexo.
    Ask,
}

impl Default for DownloadMode {
    fn default() -> Self {
        Self::Folder(files::downloads_dir())
    }
}

/// Um servidor na lista do trilho.
///
/// Cada Papo é um servidor só, então vários servidores querem dizer vários
/// backends: conta, sessão, canais e mídia separados em cada um.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ServerEntry {
    pub url: String,
    /// Nome mostrado no trilho. Começa como o endereço e passa a ser o nome
    /// de verdade assim que o servidor responde.
    #[serde(default)]
    pub label: String,
}

impl ServerEntry {
    fn new(url: String) -> Self {
        let url = normalise_server_url(&url);
        let label = host_of(&url);
        Self { url, label }
    }
}

/// O que da rede é da call: os servidores ICE, que a fazem nascer, e a
/// sinalização, que vai direto para a thread dela sem passar pelo estado da
/// tela.
fn route_call_update(ws: &mut Workspace, update: &crate::api::net::Update, ctx: &egui::Context) {
    use crate::api::net::Update;
    use crate::api::ws::Event;

    match update {
        Update::VoiceReady {
            channel_id,
            attempt,
            servers,
        } => {
            // Uma entrada que já não é a atual não monta call nenhuma — o
            // usuário desistiu antes de o ICE voltar, ou saiu e entrou de
            // novo no mesmo canal enquanto a primeira resposta vinha.
            if !ws.store.call.current(channel_id, *attempt) {
                return;
            }
            ws.call = Call::start(
                channel_id.clone(),
                IceConfig::from_servers(servers),
                ctx.clone(),
                ws.net.sender(),
            );
            ws.call_ready = false;
            if let Some(call) = &ws.call {
                let names = ws
                    .store
                    .members
                    .iter()
                    .map(|member| (member.id.clone(), member.name.clone()))
                    .collect();
                ws.net.set_event_callback(Some(call.event_callback(
                    ws.store.me.clone(),
                    names,
                )));
                #[cfg(target_os = "android")]
                crate::platform::android_call::bind(
                    call.command_sender(),
                    ws.net.sender(),
                    channel_id.clone(),
                    ws.store.call.muted,
                    ws.store.call.camera,
                    ws.store.me.clone(),
                );
            } else {
                ws.store.call.error = Some("a call não abriu".to_owned());
                ws.store.call.left();
                #[cfg(target_os = "android")]
                crate::platform::android_call::stop_service();
            }
        }
        Update::Event(event) => {
            let Some(call) = &ws.call else { return };
            // O fim da call pode vir do servidor: a sala foi destruída, a
            // sessão caiu, a permissão sumiu. O estado da tela trata disso
            // no `Store`; aqui é o pipeline que precisa ser desmontado, ou o
            // microfone continuaria aberto para ninguém.
            let mut over = false;
            let mut tell_server = false;
            match &**event {
                Event::VoiceLeft {
                    channel_id,
                    user_id,
                } if *channel_id == call.channel_id && *user_id == ws.store.me => over = true,
                Event::Failure { code, .. }
                    if code.as_deref().is_some_and(|code| {
                        crate::state::call::fatal(code, ws.store.call.phase == Phase::Joining)
                    }) =>
                {
                    over = true;
                    // Um offer inválido/codec recusado não remove o Peer no
                    // backend. Se só derrubarmos o pipeline local, a próxima
                    // entrada recebe voice-already-in-room.
                    tell_server = ws.store.call.phase == Phase::In;
                }
                _ => {}
            }
            if over {
                if tell_server {
                    leave_call(ws);
                } else {
                    ws.net.set_event_callback(None);
                    ws.call = None;
                    ws.call_ready = false;
                    ws.watching.clear();
                    #[cfg(target_os = "android")]
                    crate::platform::android_call::stop_service();
                }
            }
        }
        // O socket caiu: o servidor derruba o peer junto com a conexão que
        // pediu a entrada, então a call já acabou — só não sabíamos.
        Update::Connection(crate::api::ws::Connection::Offline) if ws.store.call.active() => {
            ws.net.set_event_callback(None);
            ws.call = None;
            ws.call_ready = false;
            ws.watching.clear();
            ws.store.call.error = None;
            ws.store.call.left();
            #[cfg(target_os = "android")]
            crate::platform::android_call::stop_service();
        }
        Update::VoiceFailed {
            channel_id,
            attempt,
            ..
        } if ws.store.call.current(channel_id, *attempt) => {
            #[cfg(target_os = "android")]
            crate::platform::android_call::stop_service();
        }
        _ => {}
    }
}

/// Sai da call: avisa o servidor, desmonta o pipeline e limpa o retrato.
fn leave_call(ws: &mut Workspace) {
    #[cfg(target_os = "android")]
    let had_call = ws.store.call.active() || ws.call.is_some();
    if ws.store.call.active() && !ws.store.call.channel_id.is_empty() {
        ws.net.send(Command::VoiceSignal(format!(
            r#"{{"type":"voice_leave","channel_id":"{}"}}"#,
            ws.store.call.channel_id
        )));
    }
    ws.net.set_event_callback(None);
    ws.call = None;
    ws.call_ready = false;
    ws.watching.clear();
    ws.camera_revision = 0;
    ws.store.call.left();
    #[cfg(target_os = "android")]
    if had_call {
        crate::platform::android_call::stop_service();
    }
}

/// O vaivém da call a cada quadro. SDP/ICE já circulam diretamente entre a
/// thread de rede e a thread da call; aqui ficam apenas estado visual e
/// seleção das câmeras que queremos receber.
fn pump_call(ws: &mut Workspace) {
    let Some(call) = &ws.call else { return };

    // A oferta só pode sair depois do `voice_joined`: antes disso o servidor
    // ainda não tem peer para receber a SDP.
    if !ws.call_ready && ws.store.call.phase == Phase::In {
        call.ready();
        ws.call_ready = true;
        // O que foi clicado enquanto a call abria vale: entramos mudos, como
        // o servidor assume, mas quem já tinha ligado a câmera não pode ficar
        // com o botão aceso e a câmera parada.
        call.set_muted(ws.store.call.muted);
        if ws.store.call.camera {
            call.set_camera(true);
        }
    }

    // Aviso que não derruba a call (sem microfone, sem câmera): aparece uma
    // vez, como os outros recados passageiros da tela.
    if let Some(warning) = call.take_warning() {
        ws.store.error = Some(warning);
    }
    // O botão da câmera segue o dispositivo, não o clique: onde não há
    // webcam — no Flatpak de hoje, por exemplo — ligar não liga nada, e ele
    // tem de voltar sozinho. O mesmo vale para a câmera que morre no meio.
    //
    // Compara-se a conta de respostas, não o valor: a tentativa que não
    // acha câmera nenhuma termina onde começou, e olhar só o valor não veria
    // resposta — o botão ficaria aceso com câmera nenhuma.
    let (revision, camera) = call.camera_state();
    if ws.camera_revision != revision {
        ws.camera_revision = revision;
        ws.store.call.camera = camera;
    }

    if call.failed() {
        let message = call.error();
        ws.store.call.error = message.clone();
        // O mesmo aviso passageiro dos outros erros: quem está na tela
        // precisa saber por que a call sumiu.
        ws.store.error = message;
        leave_call(ws);
        return;
    }

    // Quem ligou a câmera passa a ser assistido sozinho — a escolha de
    // desenho. Daqui sai só a lista de quem se quer ver; quem decide o que
    // cabe nos seis lugares é a thread da call, que é quem sabe quais estão
    // livres e reaproveita o que vaga.
    let me = ws.store.me.clone();
    let mut wanted: Vec<String> = ws
        .store
        .call
        .members()
        .iter()
        .filter(|member| member.camera_on && member.user_id != me)
        .map(|member| member.user_id.clone())
        .collect();
    wanted.sort();
    if wanted != ws.watching {
        call.watch(wanted.clone());
        ws.watching = wanted;
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// Endereço do backend. Virou o primeiro item de `servers`; fica aqui só
    /// para os ajustes gravados antes do trilho de servidores.
    #[serde(default = "default_server_url")]
    pub server_url: String,
    /// Servidores do trilho, na ordem em que aparecem.
    #[serde(default)]
    pub servers: Vec<ServerEntry>,
    /// Qual deles está na tela.
    #[serde(default)]
    pub active: usize,
    pub lang: Lang,
    pub theme: ThemePref,
    /// Vidro fosco nas barras; desligado usa fundos opacos.
    pub translucency: bool,
    pub show_members: bool,
    /// Fechar a janela esconde na bandeja em vez de encerrar.
    #[serde(default = "enabled")]
    pub close_to_tray: bool,
    #[serde(default = "enabled")]
    pub notifications: bool,
    /// O @ das novas respostas começa ligado.
    #[serde(default = "enabled")]
    pub reply_notifications: bool,
    /// Contador de menções na bandeja e na barra de tarefas.
    #[serde(default = "enabled")]
    pub badge: bool,
    /// A descrição do canal aparece ao abri-lo.
    #[serde(default = "enabled")]
    pub topic_reveal: bool,
    /// Botão de gravar recado ao lado da caixa de texto.
    #[serde(default = "enabled")]
    pub record_button: bool,
    #[serde(default)]
    pub downloads: DownloadMode,
    /// Marcas de leitura de quando havia um servidor só; migradas na
    /// primeira abertura e depois vazias.
    #[serde(default)]
    pub read_marks: std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
    /// Até quando cada canal foi visto, por servidor. É o que sobrevive ao
    /// fechamento, já que o backend registra `last_read_message` mas nunca o
    /// escreve.
    #[serde(default)]
    pub server_marks: ReadMarks,
}

/// Marcas de leitura por servidor: chave do servidor → canal → instante.
pub type ReadMarks = std::collections::HashMap<
    String,
    std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
>;

fn enabled() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server_url: default_server_url(),
            servers: Vec::new(),
            active: 0,
            lang: Lang::PtBr,
            theme: ThemePref::System,
            translucency: true,
            show_members: true,
            close_to_tray: true,
            notifications: true,
            reply_notifications: true,
            badge: true,
            topic_reveal: true,
            record_button: true,
            downloads: DownloadMode::default(),
            read_marks: std::collections::HashMap::new(),
            server_marks: ReadMarks::new(),
        }
    }
}

impl Settings {
    /// Garante que a lista de servidores existe e que o ativo aponta para um
    /// item de verdade. Ajustes gravados antes do trilho só têm `server_url`.
    fn normalise(&mut self) {
        self.server_url = normalise_server_url(&self.server_url);

        // Um servidor é identificado pelo endereço normalizado. Versões
        // anteriores deixavam o botão + criar outra entrada com o endereço
        // padrão e podiam persistir as duas. Limpa esse estado antigo já na
        // leitura, preservando qual das entradas equivalentes estava ativa.
        let had_servers = !self.servers.is_empty();
        let wanted_active = self.active.min(self.servers.len().saturating_sub(1));
        let mut unique: Vec<ServerEntry> = Vec::with_capacity(self.servers.len());
        let mut active = 0;
        for (index, mut entry) in std::mem::take(&mut self.servers).into_iter().enumerate() {
            entry.url = normalise_server_url(&entry.url);
            if let Some(existing) = unique.iter().position(|known| known.url == entry.url) {
                if index == wanted_active {
                    active = existing;
                }
                if unique[existing].label.is_empty() && !entry.label.is_empty() {
                    unique[existing].label = entry.label;
                }
                continue;
            }
            if index == wanted_active {
                active = unique.len();
            }
            unique.push(entry);
        }
        self.servers = unique;

        if self.servers.is_empty() {
            self.servers.push(ServerEntry::new(self.server_url.clone()));
            // As marcas antigas eram todas do único servidor que existia.
            if !had_servers && !self.read_marks.is_empty() {
                let key = crate::state::server_key(&self.server_url);
                self.server_marks
                    .insert(key, std::mem::take(&mut self.read_marks));
            }
            active = 0;
        }
        self.active = active.min(self.servers.len() - 1);
        self.server_url = self.servers[self.active].url.clone();
    }
}

/// Valores lidos do ambiente de trabalho.
#[derive(Clone, Debug, Default)]
pub struct SystemTheme {
    pub accent: Option<Color32>,
    pub prefers_dark: Option<bool>,
    pub animation_factor: Option<f32>,
}

impl SystemTheme {
    pub fn read() -> Self {
        Self {
            accent: desktop::system_accent(),
            prefers_dark: desktop::system_prefers_dark(),
            animation_factor: desktop::animation_factor(),
        }
    }
}

/// Um servidor conectado: rede, estado e o que a interface guarda dele.
///
/// Todos ficam ligados ao mesmo tempo — é o que faz a menção de um servidor
/// que não está na tela ainda acender o contador da bandeja.
pub struct Workspace {
    pub url: String,
    pub label: String,
    pub net: Net,
    pub store: Store,
    /// Cache durável deste processo, compartilhado por todos os servidores.
    pub cache: std::sync::Arc<ClientDb>,
    /// Partição estável deste servidor no banco.
    pub server_key: String,
    /// Dono do cache restaurado; detecta troca de conta.
    pub cached_owner: Option<String>,
    pub form: AuthForm,
    /// O pedaço da interface deste servidor, fora enquanto outro está na tela.
    pub stash: shell::Stash,
    /// Instante do último evento de digitação enviado.
    pub typing_sent: Option<std::time::Instant>,
    /// A call deste servidor, enquanto durar. Uma por servidor, e a janela
    /// só deixa uma no ar de cada vez — o microfone é um só.
    pub call: Option<Call>,
    /// Já demos o sinal verde para a oferta sair.
    call_ready: bool,
    /// De quem pedimos vídeo por último, em ordem: o pedido só sai de novo
    /// quando a lista muda.
    watching: Vec<String>,
    /// Quantas mudanças de câmera a thread da call já publicou quando
    /// olhamos pela última vez.
    camera_revision: u64,
    #[cfg(target_os = "android")]
    notification_context: std::sync::Arc<crate::platform::android_message::Context>,
    #[cfg(target_os = "android")]
    _network_registration: crate::platform::android_network::Registration,
}

impl Workspace {
    fn open(
        entry: &ServerEntry,
        marks: &ReadMarks,
        ctx: &egui::Context,
        cache: &std::sync::Arc<ClientDb>,
    ) -> Self {
        let server_key = crate::state::server_key(&entry.url);

        // Hidrata do cache antes de a rede começar. Um restore tardio poderia
        // sobrescrever estado mais novo, então ele nunca é assíncrono.
        let mut store = Store::default();
        store.read_marks = marks.get(&server_key).cloned().unwrap_or_default();
        let mut cached_owner = None;
        let secret_store = crate::storage::FileSecretStore::new();
        let has_session = secret_store
            .load(&server_key, Secret::SessionToken)
            .ok()
            .flatten()
            .is_some();
        if has_session
            && let Some(snapshot) = cache.load_snapshot(&server_key)
            && !snapshot.is_empty()
        {
            cached_owner = snapshot.owner_user_id.clone();
            store.restore_cached(snapshot);
            if let Some(owner) = cached_owner.as_deref() {
                match cache.load_outgoing(&server_key, owner) {
                    Ok(outgoing) => {
                        for item in outgoing {
                            store.project_outgoing(item);
                        }
                    }
                    Err(error) => {
                        log::warn!("outgoing {server_key}: restore inicial falhou: {error}");
                    }
                }
            }
        }

        let repaint = ctx.clone();
        let net = Net::spawn(
            entry.url.clone(),
            Wake::new(move || repaint.request_repaint()),
            std::sync::Arc::new(crate::storage::FileSecretStore::new()),
            std::sync::Arc::clone(cache),
        );

        #[cfg(target_os = "android")]
        let network_registration =
            crate::platform::android_network::register(net.sender());

        #[cfg(target_os = "android")]
        let notification_context = {
            let context = crate::platform::android_message::Context::new(
                entry.url.clone(),
                entry.label.clone(),
            );

            let message_context = std::sync::Arc::clone(&context);
            net.set_message_callback(Some(std::sync::Arc::new(move |message| {
                crate::platform::android_message::received_message(&message_context, message);
            })));

            let notification_context = std::sync::Arc::clone(&context);
            net.set_notification_callback(Some(std::sync::Arc::new(move |notification| {
                crate::platform::android_message::received(
                    &notification_context,
                    notification,
                );
            })));

            context
        };

        // A mídia usa o cookie da sessão deste servidor para baixar anexos.
        let media = Media::spawn(
            entry.url.clone(),
            std::sync::Arc::clone(&net.session),
            ctx.clone(),
        );
        Self {
            url: entry.url.clone(),
            label: entry.label.clone(),
            net,
            store,
            cache: std::sync::Arc::clone(cache),
            server_key,
            cached_owner,
            form: AuthForm {
                server_url: entry.url.clone(),
                ..AuthForm::default()
            },
            stash: shell::Stash::new(crate::media::MediaStore::new(media)),
            typing_sent: None,
            call: None,
            call_ready: false,
            watching: Vec::new(),
            camera_revision: 0,
            #[cfg(target_os = "android")]
            notification_context,
            #[cfg(target_os = "android")]
            _network_registration: network_registration,
        }
    }

    fn ensure_channel_reconciled(&mut self) {
        let Some(channel_id) = self.store.channel_needing_messages() else {
            return;
        };
        let ticket = self.store.mark_loading(&channel_id);
        self.net.send(Command::LoadMessages { ticket });
    }

    #[cfg(target_os = "android")]
    fn sync_notification_context(&self, enabled: bool, active: bool) {
        crate::platform::android_message::sync_context(
            &self.notification_context,
            enabled,
            active,
            &self.label,
            &self.store.selected_channel,
            &self.store.me,
            &self.store.my_name,
            self.store
                .channels
                .iter()
                .map(|channel| (channel.id.clone(), channel.name.clone())),
            self.store
                .members
                .iter()
                .map(|member| (member.id.clone(), member.name.clone())),
        );
    }

    /// Como o trilho vê este servidor.
    fn entry(&self, settings: &Settings) -> crate::ui::rail::Entry {
        crate::ui::rail::Entry {
            label: self.label.clone(),
            address: self.url.clone(),
            mentions: if settings.badge {
                self.store.mention_total()
            } else {
                0
            },
            unread: settings.badge && self.store.has_unread(),
            signed_in: self.store.screen == Screen::Chat,
            online: matches!(self.store.connection, crate::api::ws::Connection::Online),
        }
    }
}

pub struct PapoApp {
    workspaces: Vec<Workspace>,
    /// Um banco de cache por processo, compartilhado pelos servidores.
    cache: std::sync::Arc<ClientDb>,
    /// Índice do servidor na tela.
    active: usize,
    ui: UiState,
    settings: Settings,
    system: SystemTheme,
    tokens: Tokens,
    roles: crate::ui::roles::RolesState,
    sheet: crate::ui::settings::SettingsState,
    #[cfg(target_os = "linux")]
    tray: Option<Tray>,
    #[cfg(target_os = "linux")]
    notifier: Option<Notifier>,
    #[cfg(target_os = "linux")]
    launcher: Option<Launcher>,
    /// Diálogos do sistema em aberto (anexar, salvar como, escolher pasta).
    dialogs: Dialogs,
    /// A janela tem foco neste quadro.
    focused: bool,
    /// Sair de verdade, em vez de esconder.
    quitting: bool,
    /// Serviço do menu global; `None` fora do Linux ou sem sessão D-Bus.
    #[cfg(target_os = "linux")]
    menu: Option<GlobalMenu>,
    #[cfg(target_os = "linux")]
    appmenu: Option<AppMenuSurface>,
    #[cfg(target_os = "linux")]
    blur: Option<BlurSurface>,
    #[cfg(target_os = "linux")]
    activator: Option<Activator>,
    /// A janela está minimizada (o Wayland não deixa escondê-la de verdade).
    minimized: bool,
    window_attached: bool,
    /// Servidor de mentira: a rede é ignorada.
    demo: bool,
    /// Nós desenhamos a barra de título, porque esta área de trabalho não
    /// tem menu global para onde mandar o menu.
    own_chrome: bool,
    header: crate::ui::headerbar::HeaderState,
    /// Enquanto o servidor criado pelo botão + ainda está no modal, guarda
    /// qual servidor estava na tela para poder cancelar sem deixar lixo no trilho.
    add_server_previous: Option<usize>,
}

impl PapoApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        settings.normalise();

        #[cfg(target_os = "android")]
        if settings.notifications {
            crate::platform::android_message::ensure_permission();
        }

        let system = SystemTheme::read();
        theme::install_fonts(&cc.egui_ctx, desktop::system_ui_font().as_ref());

        let tokens = Tokens::new(appearance_for(&settings, &system), system.accent);
        theme::apply(
            &cc.egui_ctx,
            &tokens,
            settings.translucency,
            system.animation_factor.unwrap_or(1.0),
        );
        egui_extras::install_image_loaders(&cc.egui_ctx);

        #[cfg(target_os = "android")]
        cc.egui_ctx.options_mut(|options| {
            // 0.8 s (egui default) feels sluggish for a phone context menu.
            // Android's conventional long-press timing is around half a second.
            options.input_options.max_click_duration = 0.5;
        });

        let glass = cc.gl.as_ref().and_then(|gl| GlassRenderer::new(gl));
        if glass.is_none() {
            log::warn!("sem backend glow: o vidro fosco fica desligado");
        }

        // Um banco de cache por processo. Abrir aqui deixa o restore
        // acontecer antes de qualquer worker de rede subir.
        let cache = std::sync::Arc::new(ClientDb::open(crate::platform::dirs::cache_db()));

        // Todos os servidores sobem juntos: o que chega num deles enquanto
        // outro está na tela ainda conta para o contador e a notificação.
        let mut workspaces: Vec<Workspace> = settings
            .servers
            .iter()
            .map(|entry| Workspace::open(entry, &settings.server_marks, &cc.egui_ctx, &cache))
            .collect();
        let active = settings.active.min(workspaces.len() - 1);

        let demo = std::env::var("PAPO_DEMO").is_ok();
        if demo {
            // A demonstração precisa de mais de um servidor, ou o trilho não
            // mostra nada do que ele existe para mostrar.
            for (index, label) in ["Papo", "Casa", "Trabalho"].into_iter().enumerate() {
                if index >= workspaces.len() {
                    let entry = ServerEntry {
                        url: format!("https://{}.example", label.to_lowercase()),
                        label: label.to_owned(),
                    };
                    workspaces
                        .push(Workspace::open(&entry, &settings.server_marks, &cc.egui_ctx, &cache));
                }
                workspaces[index].label = label.to_owned();
            }
            crate::state::demo::seed(&mut workspaces[active].store);
            // Os outros ficam com conversa por ler, para o marcador e o
            // contador aparecerem.
            crate::state::demo::seed(&mut workspaces[1].store);
            crate::state::demo::seed(&mut workspaces[2].store);
            workspaces[1].store.read_marks.clear();
            workspaces[2].store.read_marks.clear();
        }

        // O servidor que está na tela entrega o seu guardado para a interface.
        let mut ui_state = UiState::default();
        ui_state.show_members = settings.show_members;
        ui_state.translucent = settings.translucency;
        ui_state.reveal_topic = settings.topic_reveal;
        ui_state.show_record = settings.record_button;
        ui_state.glass = glass;
        workspaces[active].stash.swap(&mut ui_state);

        Self {
            workspaces,
            cache,
            active,
            ui: ui_state,
            #[cfg(target_os = "linux")]
            tray: Tray::spawn(cc.egui_ctx.clone(), tray_labels(&settings)),
            #[cfg(target_os = "linux")]
            notifier: Notifier::spawn(),
            #[cfg(target_os = "linux")]
            launcher: Launcher::spawn(),
            dialogs: Dialogs::default(),
            focused: true,
            quitting: false,
            settings,
            system,
            tokens,
            roles: Default::default(),
            // `PAPO_SHEET=app|servidor` abre a folha de ajustes já na
            // partida. Existe para trabalhar no desenho dela sem precisar
            // clicar até lá a cada recompilação.
            sheet: {
                let mut sheet = crate::ui::settings::SettingsState::default();
                // `PAPO_SHEET=servidor:figurinhas` abre direto num painel.
                if let Ok(value) = std::env::var("PAPO_SHEET") {
                    let (surface, pane) = value.split_once(':').unwrap_or((value.as_str(), ""));
                    match surface {
                        "app" => {
                            sheet.open = Some(crate::ui::settings::Surface::App);
                            sheet.app_pane = match pane {
                                "aparencia" => crate::ui::settings::AppPane::Appearance,
                                "avisos" => crate::ui::settings::AppPane::Alerts,
                                "arquivos" => crate::ui::settings::AppPane::Files,
                                "idioma" => crate::ui::settings::AppPane::Language,
                                "sessoes" => crate::ui::settings::AppPane::Sessions,
                                _ => crate::ui::settings::AppPane::Account,
                            };
                        }
                        "servidor" | "server" => {
                            sheet.open = Some(crate::ui::settings::Surface::Server);
                            sheet.server_pane = match pane {
                                "canais" => crate::ui::settings::ServerPane::Channels,
                                "cargos" => crate::ui::settings::ServerPane::Roles,
                                "figurinhas" => crate::ui::settings::ServerPane::Emojis,
                                "auditoria" => crate::ui::settings::ServerPane::Audit,
                                _ => crate::ui::settings::ServerPane::General,
                            };
                        }
                        _ => {}
                    }
                }
                sheet
            },
            #[cfg(target_os = "linux")]
            menu: GlobalMenu::spawn(cc.egui_ctx.clone()),
            #[cfg(target_os = "linux")]
            appmenu: None,
            #[cfg(target_os = "linux")]
            blur: None,
            #[cfg(target_os = "linux")]
            activator: None,
            minimized: false,
            window_attached: false,
            demo,
            // No Android não há janela para decorar — nem barra nossa com
            // minimizar/fechar, nem menu global. Os ajustes continuam à mão
            // pela pastilha da conta, que é por onde o layout compacto abre.
            own_chrome: !cfg!(target_os = "android") && !desktop::uses_global_menu(),
            header: crate::ui::headerbar::HeaderState::default(),
            add_server_previous: None,
        }
    }

    /// O servidor que está na tela.
    fn ws(&self) -> &Workspace {
        &self.workspaces[self.active]
    }

    /// Liga a janela ao menu global e ao desfoque do compositor. Só dá para
    /// fazer depois que a janela existe, ou seja, no primeiro quadro.
    fn attach_window(&mut self, frame: &eframe::Frame) {
        if self.window_attached {
            return;
        }
        self.window_attached = true;

        #[cfg(target_os = "linux")]
        {
            use raw_window_handle::{
                HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle,
            };

            let (Ok(window), Ok(display)) = (frame.window_handle(), frame.display_handle()) else {
                return;
            };
            match (window.as_raw(), display.as_raw()) {
                (RawWindowHandle::Wayland(window), RawDisplayHandle::Wayland(display)) => {
                    if let Some(menu) = &self.menu {
                        self.appmenu = unsafe {
                            AppMenuSurface::attach(
                                display.display,
                                window.surface,
                                &menu.address.0,
                                &menu.address.1,
                            )
                        };
                    }
                    self.blur = unsafe { BlurSurface::new(display.display, window.surface) };
                    self.activator =
                        unsafe { Activator::new(display.display, window.surface) };
                }
                (RawWindowHandle::Xlib(window), _) => {
                    if let Some(menu) = &self.menu {
                        menu.register_x11(window.window as u32);
                    }
                }
                (RawWindowHandle::Xcb(window), _) => {
                    if let Some(menu) = &self.menu {
                        menu.register_x11(window.window.get());
                    }
                }
                _ => {}
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = frame;
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Traz a janela de volta. No Wayland o winit não sabe fazer isso, então
    /// pedimos ao compositor pelo xdg-activation; no X11 os comandos abaixo
    /// bastam.
    #[cfg_attr(target_os = "android", allow(dead_code))]
    fn show_window(&mut self, ctx: &egui::Context) {
        self.minimized = false;
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        #[cfg(target_os = "linux")]
        {
            if let Some(activator) = &mut self.activator {
                activator.request();
            }
            // No Wayland o compositor ignora o pedido de desminimizar; o
            // script do KWin é o que realmente traz a janela de volta.
            crate::platform::kwin::restore_window(crate::APP_ID);
        }
    }

    /// Fechar não encerra: a janela recolhe e o Papo segue na bandeja.
    #[cfg_attr(target_os = "android", allow(dead_code))]
    fn hide_window(&mut self, ctx: &egui::Context) {
        self.minimized = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }

    /// Fechar a janela só encerra quando a bandeja não está no caminho.
    #[cfg(target_os = "linux")]
    fn handle_window_lifecycle(&mut self, ctx: &egui::Context) {
        self.focused = ctx.input(|input| input.viewport().focused).unwrap_or(true);

        while let Some(command) = self.tray.as_ref().and_then(Tray::try_recv) {
            match command {
                TrayCommand::Toggle => {
                    if self.minimized || !self.focused {
                        self.show_window(ctx);
                    } else {
                        self.hide_window(ctx);
                    }
                }
                TrayCommand::Show => self.show_window(ctx),
                TrayCommand::Quit => self.quit(ctx),
            }
        }

        let closing = ctx.input(|input| input.viewport().close_requested());
        if closing && !self.quitting && self.settings.close_to_tray && self.tray.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_window(ctx);
        }

        // Menção vira número; conversa nova sem menção só acende o ícone.
        // O contador é de todos os servidores, não só do que está na tela —
        // é justamente para isso que os outros seguem conectados.
        let mentions = if self.settings.badge {
            self.workspaces
                .iter()
                .map(|ws| ws.store.mention_total())
                .sum()
        } else {
            0
        };
        let unread = self.settings.badge
            && self.workspaces.iter().any(|ws| ws.store.has_unread());

        if let Some(tray) = &self.tray {
            tray.set_badge(mentions, unread);
            tray.set_labels(tray_labels(&self.settings));
        }
        if let Some(launcher) = &self.launcher {
            launcher.set(Badge {
                count: mentions,
                urgent: unread && mentions == 0,
            });
        }
    }

    /// Avisa na área de trabalho quando chega mensagem e a janela não está à
    /// frente. A checagem acontece antes de aplicar a atualização, para ainda
    /// enxergar o estado anterior.
    #[cfg(target_os = "linux")]
    fn maybe_notify(&self, ws: &Workspace, update: &crate::api::net::Update) {
        use crate::api::net::Update;
        use crate::api::ws::Event;

        if !self.settings.notifications || (self.focused && !self.minimized) {
            return;
        }
        let Some(notifier) = &self.notifier else {
            return;
        };
        let Update::Event(event) = update else { return };
        let Event::Message(message) = &**event else {
            return;
        };
        if message.author_id == ws.store.me {
            return;
        }

        let author = ws
            .store
            .member(&message.author_id)
            .map(|member| member.name.clone())
            .unwrap_or_else(|| message.author_id.clone());
        // Com vários servidores, o canal sozinho não diz de onde veio.
        let channel = ws
            .store
            .channel(&message.channel_id)
            .map(|channel| {
                if self.workspaces.len() > 1 {
                    format!("#{} · {}", channel.name, ws.label)
                } else {
                    format!("#{}", channel.name)
                }
            })
            .unwrap_or_default();

        notifier.show(Notification {
            summary: if channel.is_empty() {
                author
            } else {
                format!("{author} · {channel}")
            },
            body: ws
                .store
                .display_mentions(message.content.as_deref().unwrap_or("")),
            tag: Some(message.channel_id.clone()),
        });
    }

    /// Entra ou cria a conta com o que está no formulário.
    fn authenticate(&mut self, register: bool, ctx: &egui::Context) {
        let index = self.active;
        let username = self.workspaces[index].form.username.trim().to_owned();
        let password = self.workspaces[index].form.password.clone();
        if username.is_empty() || password.is_empty() {
            return;
        }

        // Mudar o endereço aqui é mudar de servidor: a conexão antiga cai e
        // o item do trilho passa a apontar para o endereço novo.
        let url = normalise_server_url(&self.workspaces[index].form.server_url);
        // Um endereço vazio ou que nem vira URL deixaria o rascunho apontando
        // para o endereço interno e faria o login parecer travado.
        if url.is_empty() || url::Url::parse(&url).is_err() {
            let s = self.settings.lang.strings();
            self.workspaces[index].store.error = Some(s.invalid_server_address.to_owned());
            return;
        }

        // O mesmo backend não pode ocupar duas posições do trilho: sessão,
        // senha do servidor e marcas locais já são todos indexados pela URL.
        // Se o endereço digitado no modal já existe, descarta o rascunho,
        // reaproveita a entrada real e continua o login nela com o formulário
        // que o usuário acabou de preencher.
        if self.add_server_previous.is_some()
            && let Some(existing) = self
                .workspaces
                .iter()
                .enumerate()
                .find(|(other, ws)| *other != index && normalise_server_url(&ws.url) == url)
                .map(|(other, _)| other)
        {
            let form = self.workspaces[index].form.clone();
            self.cancel_add_server(ctx);
            self.activate(existing, ctx);
            self.workspaces[existing].form = AuthForm {
                server_url: url,
                ..form
            };
            self.authenticate(register, ctx);
            return;
        }

        if url != self.workspaces[index].url {
            self.reopen(index, url, ctx);
        }

        let ws = &mut self.workspaces[index];
        ws.store.busy = true;
        ws.store.error = None;
        ws.net.send(if register {
            Command::Register { username, password }
        } else {
            Command::Login { username, password }
        });
    }

    /// Manda a senha do servidor, para servidores fechados.
    fn unlock_server(&mut self) {
        let index = self.active;
        let password = self.workspaces[index].form.server_password.clone();
        if password.is_empty() {
            return;
        }
        self.workspaces[index].store.busy = true;
        self.workspaces[index].store.error = None;
        self.workspaces[index]
            .net
            .send(Command::LoginServer { password });
    }

    /// Reabre um servidor num endereço novo, jogando fora a conexão antiga.
    fn reopen(&mut self, index: usize, url: String, ctx: &egui::Context) {
        let form = self.workspaces[index].form.clone();
        let old_key = self.workspaces[index].server_key.clone();
        let entry = ServerEntry::new(url);
        let mut fresh = Workspace::open(&entry, &self.settings.server_marks, ctx, &self.cache);
        fresh.form = AuthForm {
            server_url: entry.url.clone(),
            ..form
        };
        // O endereço velho some de propósito: o cache dele não pode aparecer
        // sob a chave nova só porque o servidor é o mesmo.
        self.cache.clear_server(&old_key);
        self.settings.servers[index] = entry;
        // O servidor na tela devolve o guardado para o substituto, ou a
        // interface ficaria com a mídia de uma conexão que já morreu.
        if index == self.active {
            self.workspaces[index].stash.swap(&mut self.ui);
            fresh.stash.swap(&mut self.ui);
        }
        self.workspaces[index] = fresh;
    }

    /// Passa a mostrar outro servidor. Os dois seguem conectados; o que troca
    /// é qual deles ocupa a janela.
    fn activate(&mut self, index: usize, ctx: &egui::Context) {
        if index == self.active || index >= self.workspaces.len() {
            return;
        }
        // O que estava na tela recolhe o seu; o novo entrega o dele.
        let previous = self.active;
        self.workspaces[previous].stash.swap(&mut self.ui);
        self.workspaces[index].stash.swap(&mut self.ui);
        self.active = index;
        self.settings.active = index;
        self.settings.server_url = self.workspaces[index].url.clone();
        // A mídia do servidor que saiu para de tocar junto com ele.
        self.workspaces[previous].stash.media.pause_all();
        ctx.request_repaint();
    }

    /// Abre "Adicionar servidor" como rascunho cancelável.
    fn add_server(&mut self, ctx: &egui::Context) {
        // O novo servidor é provisório até a autenticação terminar. Se o
        // usuário clicar fora do cartão, voltamos exatamente para quem estava
        // ativo e descartamos este workspace.
        let previous = self.active;
        let entry = ServerEntry::new(DRAFT_SERVER_URL.to_owned());
        let mut workspace = Workspace::open(&entry, &self.settings.server_marks, ctx, &self.cache);
        // Não herda o endereço padrão nem a sessão dele. O cartão nasce
        // realmente vazio e só cria conexão com o servidor digitado no envio.
        workspace.form.server_url.clear();
        workspace.store.screen = Screen::Auth;
        self.settings.servers.push(entry);
        self.workspaces.push(workspace);
        self.activate(self.workspaces.len() - 1, ctx);
        self.add_server_previous = Some(previous);
    }

    fn cancel_add_server(&mut self, ctx: &egui::Context) {
        let Some(previous) = self.add_server_previous.take() else {
            return;
        };
        let index = self.active;
        if index >= self.workspaces.len() || index == previous {
            return;
        }

        // Recolhe o estado visual do rascunho, remove-o e devolve o estado
        // visual do servidor que estava aberto antes do +.
        self.workspaces[index].stash.swap(&mut self.ui);
        let key = crate::state::server_key(&self.workspaces[index].url);
        self.settings.server_marks.remove(&key);
        self.workspaces[index].cache.clear_server(&key);
        self.workspaces[index].net.forget_credentials();
        self.workspaces.remove(index);
        self.settings.servers.remove(index);

        self.active = previous.min(self.workspaces.len() - 1);
        self.settings.active = self.active;
        self.settings.server_url = self.workspaces[self.active].url.clone();
        self.workspaces[self.active].stash.swap(&mut self.ui);
        ctx.request_repaint();
    }

    fn clicked_outside_auth_card(ctx: &egui::Context, rect: egui::Rect) -> bool {
        ctx.input(|input| {
            input.pointer.any_pressed()
                && input
                    .pointer
                    .interact_pos()
                    .is_some_and(|position| !rect.contains(position))
        })
    }

    /// Tira um servidor do trilho. A conta no servidor continua existindo; o
    /// que sai é a conexão e o que ela guardava aqui.
    fn remove_server(&mut self, index: usize, ctx: &egui::Context) {
        // O último não sai: sem nenhum servidor não há para onde ir.
        if self.workspaces.len() <= 1 || index >= self.workspaces.len() {
            return;
        }
        // Só o servidor que está na tela tem o seu guardado na interface.
        let was_active = index == self.active;
        if was_active {
            self.workspaces[index].stash.swap(&mut self.ui);
        }
        let key = crate::state::server_key(&self.workspaces[index].url);
        self.settings.server_marks.remove(&key);
        self.workspaces[index].cache.clear_server(&key);
        self.workspaces[index].net.forget_credentials();
        self.workspaces.remove(index);
        self.settings.servers.remove(index);

        let active = if self.active > index {
            self.active - 1
        } else {
            self.active.min(self.workspaces.len() - 1)
        };
        self.active = active;
        self.settings.active = active;
        self.settings.server_url = self.workspaces[active].url.clone();
        if was_active {
            self.workspaces[active].stash.swap(&mut self.ui);
        }
        ctx.request_repaint();
    }

    /// Ações da conversa: mensagens novas, digitação e carga sob demanda.
    fn pump_chat(&mut self) {
        if self.demo {
            return;
        }
        // Só o servidor na tela: carregar mensagem de canal que ninguém está
        // olhando não ajuda ninguém, e os outros seguem recebendo os eventos.
        let focused = self.focused && !self.minimized;
        let typed = self.ui.typed;
        let ws = &mut self.workspaces[self.active];

        ws.ensure_channel_reconciled();

        // Com a janela à frente, o canal aberto está sendo lido agora.
        if focused && !ws.store.selected_channel.is_empty() {
            let channel_id = ws.store.selected_channel.clone();
            ws.store.mark_read(&channel_id);
            // O servidor também precisa saber, ou a menção volta no próximo
            // dispositivo.
            let ids = ws.store.take_open_notifications(&channel_id);
            if !ids.is_empty() && !ws.store.me.is_empty() {
                #[cfg(target_os = "android")]
                crate::platform::android_message::clear_channel(&ws.url, &channel_id);
                ws.net.send(Command::MarkNotificationsRead {
                    user_id: ws.store.me.clone(),
                    ids,
                });
            }
        }

        // Um evento de digitação a cada três segundos basta para o servidor.
        if typed {
            let now = std::time::Instant::now();
            let stale = ws
                .typing_sent
                .is_none_or(|last| now.duration_since(last).as_secs() >= 3);
            if stale && !ws.store.selected_channel.is_empty() {
                ws.typing_sent = Some(now);
                ws.net.send(Command::Typing {
                    channel_id: ws.store.selected_channel.clone(),
                });
            }
        }
    }

    /// Executa o que a conversa pediu.
    fn handle_chat(&mut self, ctx: &egui::Context, action: ChatAction) {
        // Administração de canal é navegação de UI, não mutação. Todos os
        // pontos de entrada convergem para Ajustes do servidor → Canais.
        match &action {
            ChatAction::NewChannel => {
                self.sheet.open_new_channel();
                return;
            }
            ChatAction::EditChannel(id) => {
                if let Some(channel) = self.workspaces[self.active].store.channel(id).cloned() {
                    self.sheet.open_edit_channel(&channel);
                }
                return;
            }
            ChatAction::RequestDeleteChannel(id) => {
                if let Some(channel) = self.workspaces[self.active].store.channel(id).cloned() {
                    self.sheet.open_delete_channel(&channel);
                }
                return;
            }
            _ => {}
        }

        // Na demonstração as ações mexem só no estado local.
        if self.demo {
            self.handle_chat_demo(ctx, action);
            return;
        }
        // Entrar numa call mexe em todos os servidores, não só no que está na
        // tela: o microfone é um só. Sem isto, entrar no servidor A, trocar
        // para o B e entrar lá deixava os dois mandando a sua voz, cada um
        // com o seu pipeline — e o trilho não mostra nem qual deles era.
        if let ChatAction::JoinVoice(channel_id) = action {
            #[cfg(target_os = "android")]
            {
                use crate::platform::permission::{self, Status};
                if permission::ensure(permission::RECORD_AUDIO) == Status::Asking {
                    return;
                }
            }
            for ws in &mut self.workspaces {
                leave_call(ws);
            }
            #[cfg(target_os = "android")]
            {
                let title = self.workspaces[self.active]
                    .store
                    .channel(&channel_id)
                    .map(|channel| channel.name.clone())
                    .unwrap_or_else(|| "Papo".to_owned());
                crate::platform::android_call::start_service(&title);
            }
            let ws = &mut self.workspaces[self.active];
            ws.store.selected_channel = channel_id.clone();
            let attempt = ws.store.call.joining(channel_id.clone());
            ws.net.send(Command::JoinVoice {
                channel_id,
                attempt,
            });
            return;
        }
        let s = self.settings.lang.strings();
        let ws = &mut self.workspaces[self.active];
        match action {
            ChatAction::Send {
                content,
                reply_to,
                notify_reply,
                attachments,
            } => {
                let channel_id = ws.store.selected_channel.clone();
                let wire_content = content;
                // Sem canal não há para onde mandar. Engolir a mensagem aqui
                // fazia o envio parecer quebrado: a caixa esvaziava e nada
                // acontecia, sem uma palavra de explicação.
                if channel_id.is_empty() {
                    ws.store.error = Some(s.no_channel_selected.to_owned());
                    if attachments.is_empty() {
                        let (visible, bindings) =
                            ws.store.display_mentions_with_bindings(&wire_content);
                        self.ui.composer = visible;
                        self.ui.composer_mentions = bindings;
                        self.ui.replying = reply_to;
                        self.ui.reply_notify = notify_reply;
                    }
                    return;
                }
                // Texto comum entra na fila persistente. O Net só publica o
                // eco depois que o ClientDb confirmou a linha; anexo continua
                // no caminho legado porque o backend define os metadados do
                // upload e caminhos locais não podem ir para o banco.
                if attachments.is_empty() {
                    let owner_user_id = ws.store.me.clone();
                    if owner_user_id.is_empty() {
                        ws.store.error =
                            Some("sessão ainda não verificada para enviar".to_owned());
                        let (visible, bindings) =
                            ws.store.display_mentions_with_bindings(&wire_content);
                        self.ui.composer = visible;
                        self.ui.composer_mentions = bindings;
                        self.ui.replying = reply_to;
                        self.ui.reply_notify = notify_reply;
                        return;
                    }
                    ws.net.send(Command::QueueMessage {
                        local_id: papo_core::cache::new_local_id(),
                        owner_user_id,
                        channel_id,
                        content: wire_content,
                        reply_to,
                        notify_reply,
                        created_at: papo_core::cache::now_millis(),
                    });
                } else {
                    ws.net.send(Command::SendMessage {
                        channel_id,
                        content: wire_content,
                        reply_to,
                        notify_reply,
                        attachments,
                    });
                }
            }
            ChatAction::Edit {
                message_id,
                content,
            } => ws.net.send(Command::EditMessage {
                message_id,
                content,
            }),
            ChatAction::Delete(message_id) => {
                ws.net.send(Command::DeleteMessage { message_id })
            }
            ChatAction::React {
                message_id,
                emoji,
                add,
            } => {
                let channel_id = ws
                    .store
                    .message(&message_id)
                    .map(|message| message.channel_id.clone())
                    .unwrap_or_else(|| ws.store.selected_channel.clone());
                // A reação aparece na hora; o contador certo vem pelo evento.
                ws.store.set_reaction_local(&message_id, &emoji, add);
                ws.net.send(Command::React {
                    channel_id,
                    message_id,
                    emoji: emoji.request(),
                    add,
                });
            }
            ChatAction::Pin { message_id, pin } => {
                let channel_id = ws
                    .store
                    .message(&message_id)
                    .map(|message| message.channel_id.clone())
                    .unwrap_or_else(|| ws.store.selected_channel.clone());
                ws.net.send(Command::Pin {
                    channel_id,
                    message_id,
                    pin,
                });
            }
            ChatAction::Download { id, name } => match self.settings.downloads.clone() {
                DownloadMode::Ask => {
                    self.dialogs
                        .save_as(ctx.clone(), id, name, files::downloads_dir())
                }
                DownloadMode::Folder(dir) => {
                    let dir = if dir.is_dir() {
                        dir
                    } else {
                        files::downloads_dir()
                    };
                    let dest = files::unique_path(&dir, &name);
                    self.ui.media.save(&id, &name, dest);
                }
            },
            ChatAction::NewChannel
            | ChatAction::EditChannel(_)
            | ChatAction::RequestDeleteChannel(_) => unreachable!("tratadas antes do match"),
            ChatAction::CreateChannel { name, kind, topic } => {
                ws.net.send(Command::CreateChannel { name, kind, topic })
            }
            ChatAction::UpdateChannel {
                channel_id,
                name,
                topic,
            } => ws.net.send(Command::UpdateChannel {
                channel_id,
                name,
                topic,
            }),
            ChatAction::DeleteChannel(channel_id) => {
                ws.net.send(Command::DeleteChannel { channel_id })
            }
            ChatAction::ChannelNotifications {
                channel_id,
                setting,
            } => ws.net.send(Command::SetChannelNotifications {
                channel_id,
                setting: setting.to_owned(),
            }),
            ChatAction::MoveChannel {
                channel_id,
                old_position,
                new_position,
            } => ws.net.send(Command::MoveChannel {
                channel_id,
                old_position,
                new_position,
            }),
            ChatAction::BanUser { user_id, banned } => {
                ws.net.send(Command::BanUser { user_id, banned })
            }
            ChatAction::ResetUser(user_id) => ws.net.send(Command::ResetUser { user_id }),
            // Entrar já foi tratado antes do `match`, porque mexe em todos
            // os servidores de uma vez.
            ChatAction::JoinVoice(_) => {}
            ChatAction::LeaveVoice => leave_call(ws),
            ChatAction::ToggleMute => {
                let muted = !ws.store.call.muted;
                ws.store.call.muted = muted;
                if let Some(call) = &ws.call {
                    call.set_muted(muted);
                }
            }
            ChatAction::ToggleCamera => {
                let on = !ws.store.call.camera;
                ws.store.call.camera = on;
                if let Some(call) = &ws.call {
                    call.set_camera(on);
                }
            }
            ChatAction::CollapseCall(collapsed) => ws.store.call.collapsed = collapsed,
            ChatAction::FloatCall(floating) => {
                ws.store.call.floating = floating;
                ws.store.call.collapsed = floating;
                if floating {
                    ws.store.call.popped_out = false;
                }
            }
            // Voltar para a call é ir ao canal dela, como o clique que
            // levou na primeira vez — e abrir a folha se estava encolhida.
            ChatAction::OpenCall => {
                if !ws.store.call.channel_id.is_empty() {
                    ws.store.selected_channel = ws.store.call.channel_id.clone();
                    ws.store.call.collapsed = false;
                    ws.store.call.floating = false;
                }
            }
            ChatAction::PopOutCall(out) => {
                ws.store.call.popped_out = out;
                if out {
                    ws.store.call.floating = false;
                }
            }
            ChatAction::Search(text) => ws.net.send(Command::Search { text }),
            ChatAction::PickFiles => self.dialogs.pick_files(ctx.clone()),
            ChatAction::PickGif => self.dialogs.pick_animations(ctx.clone()),
            ChatAction::OpenExternally(path) => files::open_path(&path),
        }
    }

    /// A demonstração responde sozinha: sem rede, o estado é a verdade.
    fn handle_chat_demo(&mut self, ctx: &egui::Context, action: ChatAction) {
        let ws = &mut self.workspaces[self.active];
        match action {
            ChatAction::Send { content, reply_to, .. } => {
                let channel_id = ws.store.selected_channel.clone();
                let message_id = ws.store.push_pending(&channel_id, &content, reply_to);
                ws.store.set_message_pending_local(&message_id, false);
            }
            ChatAction::Edit {
                message_id,
                content,
            } => {
                ws.store.edit_message_local(&message_id, content);
            }
            ChatAction::Delete(message_id) => {
                ws.store.delete_message_local(&message_id);
            }
            ChatAction::React {
                message_id,
                emoji,
                add,
            } => {
                ws.store.set_reaction_local(&message_id, &emoji, add);
            }
            ChatAction::Pin { message_id, pin } => {
                ws.store.set_message_pinned_local(&message_id, pin);
            }
            ChatAction::Download { id, name } => match self.settings.downloads.clone() {
                DownloadMode::Ask => {
                    self.dialogs
                        .save_as(ctx.clone(), id, name, files::downloads_dir())
                }
                DownloadMode::Folder(dir) => {
                    let dir = if dir.is_dir() { dir } else { files::downloads_dir() };
                    let dest = files::unique_path(&dir, &name);
                    self.ui.media.save(&id, &name, dest);
                }
            },
            ChatAction::NewChannel
            | ChatAction::EditChannel(_)
            | ChatAction::RequestDeleteChannel(_) => unreachable!("tratadas antes do match"),
            // Na demonstração não há rede: o canal nasce, muda e some aqui
            // mesmo, para o editor inline poder ser testado sem backend.
            ChatAction::CreateChannel { name, kind, topic } => {
                use crate::state::{Channel, ChannelKind};
                let kind = match kind.as_str() {
                    "voice" => ChannelKind::Voice,
                    "category" => ChannelKind::Category,
                    _ => ChannelKind::Text,
                };
                let position = ws.store.channels.len() as i32;
                let id = format!("demo-channel-{position}");
                ws.store.channels.push(Channel {
                    id: id.clone(),
                    name,
                    kind,
                    topic,
                    position,
                    unread: false,
                    mentions: 0,
                });
                if kind == ChannelKind::Text {
                    ws.store.selected_channel = id;
                }
            }
            ChatAction::UpdateChannel {
                channel_id,
                name,
                topic,
            } => {
                if let Some(channel) = ws
                    .store
                    .channels
                    .iter_mut()
                    .find(|channel| channel.id == channel_id)
                {
                    channel.name = name;
                    channel.topic = topic;
                }
            }
            // A call de mentira não abre microfone nenhum: serve para o
            // desenho da grade, da pastilha e da folha.
            ChatAction::JoinVoice(channel_id) => {
                ws.store.selected_channel = channel_id.clone();
                ws.store.call.joining(channel_id.clone());
                ws.store.call.joined(
                    &channel_id,
                    crate::state::demo::call_members(),
                    vec!["u-ana".to_owned()],
                );
            }
            ChatAction::LeaveVoice => ws.store.call.left(),
            ChatAction::ToggleMute => ws.store.call.muted = !ws.store.call.muted,
            ChatAction::ToggleCamera => ws.store.call.camera = !ws.store.call.camera,
            ChatAction::CollapseCall(collapsed) => ws.store.call.collapsed = collapsed,
            ChatAction::FloatCall(floating) => {
                ws.store.call.floating = floating;
                ws.store.call.collapsed = floating;
                if floating {
                    ws.store.call.popped_out = false;
                }
            }
            // Voltar para a call é ir ao canal dela, como o clique que
            // levou na primeira vez — e abrir a folha se estava encolhida.
            ChatAction::OpenCall => {
                if !ws.store.call.channel_id.is_empty() {
                    ws.store.selected_channel = ws.store.call.channel_id.clone();
                    ws.store.call.collapsed = false;
                }
            }
            ChatAction::PopOutCall(out) => ws.store.call.popped_out = out,
            ChatAction::DeleteChannel(id) => {
                ws.store.channels.retain(|channel| channel.id != id);
                if ws.store.selected_channel == id {
                    ws.store.selected_channel = ws
                        .store
                        .channels
                        .first()
                        .map(|channel| channel.id.clone())
                        .unwrap_or_default();
                }
            }
            // Sem rede na demonstração: estas ações não têm efeito local.
            ChatAction::ChannelNotifications { .. }
            | ChatAction::MoveChannel { .. }
            | ChatAction::BanUser { .. }
            | ChatAction::ResetUser(_)
            | ChatAction::Search(_) => {}
            ChatAction::PickFiles => self.dialogs.pick_files(ctx.clone()),
            ChatAction::PickGif => self.dialogs.pick_animations(ctx.clone()),
            ChatAction::OpenExternally(path) => files::open_path(&path),
        }
    }

    /// Lê o que chegou de cada servidor. Todos são atendidos no mesmo
    /// quadro: um servidor que não está na tela ainda precisa contar as
    /// menções e disparar a notificação.
    fn pump_network(&mut self, ctx: &egui::Context) {
        // No modo demonstração a rede não manda no estado.
        if self.demo {
            for ws in &self.workspaces {
                while ws.net.try_recv().is_some() {}
            }
            return;
        }

        for index in 0..self.workspaces.len() {
            while let Some(update) = self.workspaces[index].net.try_recv() {
                #[cfg(target_os = "linux")]
                self.maybe_notify(&self.workspaces[index], &update);

                // Reconectou: o que aconteceu durante a queda vem da carga
                // nova.
                let reconnected = matches!(
                    update,
                    crate::api::net::Update::Connection(crate::api::ws::Connection::Online)
                );
                // O portão do servidor abriu: entra com o que já está no
                // formulário, em vez de fazer o usuário clicar de novo.
                let unlocked = matches!(update, crate::api::net::Update::ServerUnlocked);
                let session_me = match &update {
                    crate::api::net::Update::Session(Some(me)) => Some(me.id.clone()),
                    _ => None,
                };
                let session_ended =
                    matches!(update, crate::api::net::Update::Session(None));
                let rejected_outgoing = match &update {
                    crate::api::net::Update::OutgoingRejected {
                        content,
                        reply_to,
                        notify_reply,
                        ..
                    } => Some((content.clone(), reply_to.clone(), *notify_reply)),
                    _ => None,
                };
                let ws = &mut self.workspaces[index];
                route_call_update(ws, &update, ctx);
                if reconnected && ws.store.screen == Screen::Chat {
                    ws.net.send(Command::Refresh);
                }
                if unlocked {
                    let username = ws.form.username.trim().to_owned();
                    let password = ws.form.password.clone();
                    if !username.is_empty() && !password.is_empty() {
                        ws.store.busy = true;
                        ws.net.send(Command::Login { username, password });
                    }
                }
                ws.store.apply(update);

                // O nome de verdade do servidor substitui o host no trilho
                // assim que ele chega.
                if let Some(server) = &ws.store.server
                    && !server.name.is_empty()
                    && ws.label != server.name
                {
                    ws.label = server.name.clone();
                    self.settings.servers[index].label = server.name.clone();
                }

                // Conta verificada diferente da dona do cache: a conversa
                // antiga não pode aparecer para o novo login.
                if let Some(me_id) = session_me {
                    if ws
                        .cached_owner
                        .as_deref()
                        .is_some_and(|owner| owner != me_id)
                    {
                        // Troca de conta limpa só o cache reconstruível.
                        // Intenções não enviadas continuam particionadas pelo
                        // owner antigo e nunca são projetadas para esta conta.
                        ws.cache.clear_cached_data(&ws.server_key);
                        ws.store.clear_cached_state();
                    }
                    ws.cached_owner = Some(me_id);
                }
                if session_ended {
                    ws.cached_owner = None;
                }

                // Toda mutação Live/Reconcile vira efeito de cache aqui.
                let ops = ws.store.take_cache_ops();
                if !ops.is_empty() {
                    ws.cache.submit(&ws.server_key, ops);
                }

                if index == self.active
                    && let Some((content, reply_to, notify_reply)) = rejected_outgoing
                {
                    let (visible, bindings) = ws.store.display_mentions_with_bindings(&content);
                    // O editor tinha sido limpo no submit; uma falha da
                    // persistência restaura o texto porque nenhum POST saiu.
                    self.ui.composer = visible;
                    self.ui.composer_mentions = bindings;
                    self.ui.replying = reply_to;
                    self.ui.reply_notify = notify_reply;
                }
            }
        }
    }

    #[cfg(target_os = "android")]
    fn handle_android_notification_navigation(&mut self, ctx: &egui::Context) {
        let Some(target) = crate::platform::android_message::take_navigation() else {
            return;
        };
        let wanted = normalise_server_url(&target.server_url);
        let Some(index) = self
            .workspaces
            .iter()
            .position(|ws| normalise_server_url(&ws.url) == wanted)
        else {
            return;
        };

        let ready = self.workspaces[index]
            .store
            .channels
            .iter()
            .any(|channel| channel.id == target.channel_id);
        if !ready {
            crate::platform::android_message::defer_navigation(target);
            return;
        }

        self.activate(index, ctx);
        let ws = &mut self.workspaces[index];
        ws.store.selected_channel = target.channel_id;
        self.ui.mobile_surface = crate::ui::shell::MobileSurface::Chat;
        self.ui.jump = Some(crate::ui::shell::Jump {
            message_id: target.message_id,
            found: None,
            since: ctx.input(|input| input.time),
        });
        ctx.request_repaint();
    }

    /// A call numa janela só dela. É uma viewport de verdade, não um
    /// diálogo dentro da janela: o pedido era poder jogá-la noutro monitor,
    /// e para isso ela precisa ser uma janela que o compositor conheça.
    fn call_window(&mut self, ctx: &egui::Context) {
        let active = self.active;
        #[cfg(target_os = "android")]
        {
            return;
        }
        #[cfg(not(target_os = "android"))]
        if !self.workspaces[active].store.call.popped_out {
            return;
        }
        let t = self.tokens;
        let s = self.settings.lang.strings();
        let interface = &mut self.ui;
        let ws = &mut self.workspaces[active];
        let mut closing = false;

        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("papo-call"),
            egui::ViewportBuilder::default()
                .with_title(s.call_window_title)
                .with_app_id(crate::APP_ID)
                .with_inner_size([760.0, 520.0])
                .with_min_inner_size([360.0, 280.0]),
            |ctx, _class| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(t.content_bg))
                    .show(ctx, |ui| {
                        crate::ui::call::window(
                            ui,
                            &ws.store,
                            interface,
                            ws.call.as_mut(),
                            &t,
                            s,
                        );
                    });
                if ctx.input(|input| input.viewport().close_requested()) {
                    closing = true;
                }
            },
        );

        // Fechar a janela traz a call de volta para dentro; ela não morre
        // junto, como não morreria se você tivesse encolhido a folha.
        if closing {
            ws.store.call.popped_out = false;
        }
    }

    /// Barra de título própria, onde não existe menu global.
    fn draw_header(&mut self, ui: &mut egui::Ui) {
        if !self.own_chrome {
            return;
        }
        let model = build_menu(&self.settings);
        // O nome do servidor na tela é o que a barra tem de mais útil a
        // dizer; sem sessão, sobra o nome do aplicativo.
        let title = match &self.ws().store.server {
            Some(server) if !server.name.is_empty() => {
                format!("Papo — {}", server.name)
            }
            _ => "Papo".to_owned(),
        };
        let commands =
            crate::ui::headerbar::draw(ui, &mut self.header, &model, &title, &self.tokens);
        self.ui.pending.extend(commands);
    }

    /// Trilho de servidores e o que ele pediu.
    fn draw_rail(&mut self, ui: &mut egui::Ui, s: &'static crate::i18n::Strings, ctx: &egui::Context) {
        let entries: Vec<_> = self
            .workspaces
            .iter()
            .map(|ws| ws.entry(&self.settings))
            .collect();
        if let Some(action) = crate::ui::rail::draw(ui, &entries, self.active, &self.tokens, s) {
            self.handle_rail_action(action, ctx);
        }
    }

    fn handle_rail_action(&mut self, action: crate::ui::rail::RailAction, ctx: &egui::Context) {
        match action {
            crate::ui::rail::RailAction::Select(index) => {
                self.add_server_previous = None;
                self.activate(index, ctx);
                self.ui.mobile_surface = crate::ui::shell::MobileSurface::Chat;
            }
            crate::ui::rail::RailAction::Add => self.add_server(ctx),
            crate::ui::rail::RailAction::Remove(index) => self.remove_server(index, ctx),
        }
    }

    /// Respostas dos diálogos do sistema e arquivos soltos na janela.
    fn pump_files(&mut self, ctx: &egui::Context) {
        for chosen in self.dialogs.poll() {
            match chosen {
                Chosen::Files(uploads) => self.ui.attachments.extend(uploads),
                Chosen::Folder(path) => self.settings.downloads = DownloadMode::Folder(path),
                Chosen::SaveAs { id, name, dest } => self.ui.media.save(&id, &name, dest),
                Chosen::Image {
                    purpose,
                    blob,
                    format,
                    shrunk,
                } => match purpose {
                    ImagePick::Avatar => {
                        self.workspaces[self.active]
                            .net
                            .send(Command::SetAvatar { blob, format });
                    }
                    // A figurinha espera pelo nome: ela aparece no painel com
                    // um campo ao lado, e só então sobe.
                    ImagePick::Sticker => {
                        self.sheet.draft.pending_sticker =
                            Some(crate::ui::settings::PendingSticker {
                                blob,
                                format,
                                shrunk,
                                name: String::new(),
                            });
                        self.sheet.open = Some(crate::ui::settings::Surface::Server);
                        self.sheet.server_pane = crate::ui::settings::ServerPane::Emojis;
                    }
                },
                Chosen::Cancelled => {}
            }
        }

        // Arrastar e soltar entra na mesma fila do seletor.
        let dropped = ctx.input(|input| input.raw.dropped_files.clone());
        for file in dropped {
            let path = file.path();
            if !path.as_os_str().is_empty() {
                self.ui.attachments.push(files::describe(&path.to_path_buf()));
            }
        }
    }

    /// Mantém o menu do painel em dia com o estado da aplicação.
    #[cfg(target_os = "linux")]
    fn sync_menu(&mut self) {
        let model = build_menu(&self.settings);
        if let Some(menu) = &mut self.menu {
            menu.set_model(model);
            while let Some(command) = menu.try_recv() {
                self.ui.pending.push(command);
            }
        }
    }

    /// Pede ao compositor que desfoque a área de trabalho por trás das
    /// laterais translúcidas.
    #[cfg(target_os = "linux")]
    fn sync_blur_regions(&mut self, ctx: &egui::Context) {
        let Some(blur) = &mut self.blur else {
            return;
        };
        if !self.settings.translucency {
            blur.set_regions(&[]);
            return;
        }
        let screen = ctx.content_rect();
        let left = crate::ui::rail::RAIL_WIDTH + crate::ui::shell::SIDEBAR_WIDTH;
        // A nossa barra de título é opaca: o desfoque começa abaixo dela.
        let top = if self.own_chrome {
            crate::ui::headerbar::HEADER_HEIGHT as i32
        } else {
            0
        };
        let height = screen.height() as i32 - top;
        let mut regions = vec![(0, top, left as i32, height)];
        if self.settings.show_members {
            let width = crate::ui::shell::MEMBERS_WIDTH as i32;
            regions.push((screen.width() as i32 - width, top, width, height));
        }
        blur.set_regions(&regions);
    }

    fn retheme(&mut self, ctx: &egui::Context) {
        self.tokens = Tokens::new(
            appearance_for(&self.settings, &self.system),
            self.system.accent,
        );
        self.ui.translucent = self.settings.translucency;
        theme::apply(
            ctx,
            &self.tokens,
            self.settings.translucency,
            self.system.animation_factor.unwrap_or(1.0),
        );
    }

    fn handle(&mut self, ctx: &egui::Context, command: MenuCommand) {
        match command {
            MenuCommand::ToggleMembers => {
                self.settings.show_members = !self.settings.show_members;
                self.ui.show_members = self.settings.show_members;
            }
            MenuCommand::ToggleTranslucency => {
                self.settings.translucency = !self.settings.translucency;
                self.retheme(ctx);
            }
            MenuCommand::ToggleNotifications => {
                self.settings.notifications = !self.settings.notifications;
            }
            MenuCommand::ToggleBadge => {
                self.settings.badge = !self.settings.badge;
            }
            MenuCommand::ToggleTopicReveal => {
                self.settings.topic_reveal = !self.settings.topic_reveal;
                self.ui.reveal_topic = self.settings.topic_reveal;
            }
            MenuCommand::ToggleRecordButton => {
                self.settings.record_button = !self.settings.record_button;
                self.ui.show_record = self.settings.record_button;
            }
            MenuCommand::ToggleCloseToTray => {
                self.settings.close_to_tray = !self.settings.close_to_tray;
            }
            // O X da nossa barra faz o que o X do sistema faria.
            MenuCommand::CloseWindow => {
                #[cfg(target_os = "linux")]
                if self.settings.close_to_tray && self.tray.is_some() {
                    self.hide_window(ctx);
                    return;
                }
                self.quit(ctx);
            }
            MenuCommand::SignOut => self.ws().net.send(Command::Logout),
            MenuCommand::Preferences => self
                .sheet
                .toggle(crate::ui::settings::Surface::App),
            MenuCommand::SwitchLanguage(lang) => self.settings.lang = lang,
            MenuCommand::Quit => self.quit(ctx),
            // Marcar tudo como lido limpa todos os servidores: é o que o
            // contador da bandeja está somando.
            MenuCommand::MarkAllRead => {
                for ws in &mut self.workspaces {
                    ws.store.mark_all_read();
                }
            }
            MenuCommand::NewChannel => self.sheet.open_new_channel(),
            // A busca virou parte da pastilha de ações, dentro da conversa.
            MenuCommand::Search => {
                crate::ui::shell::toggle_panel(&mut self.ui, crate::ui::shell::PanelKind::Search)
            }
            // "Sobre" virou uma linha no fim dos ajustes, em vez de uma
            // janela só para dizer a versão.
            MenuCommand::About => {
                self.sheet.toggle(crate::ui::settings::Surface::App);
                self.sheet.app_pane = crate::ui::settings::AppPane::Sessions;
            }
            // Cargos e o resto do servidor moram na folha do servidor.
            MenuCommand::Roles | MenuCommand::ServerSettings => {
                self.sheet.toggle(crate::ui::settings::Surface::Server);
                if self.sheet.open.is_some() {
                    let ws = &self.workspaces[self.active];
                    ws.net.send(Command::LoadRoles);
                    ws.net.send(Command::LoadAuditLogs);
                }
            }
        }
    }

    /// Janela de busca: termo em cima, resultados embaixo. Clicar num
    /// resultado abre o canal dele.
     /// Tela de cargos. Ela só descreve o que quer; a tradução em comandos
    /// de rede é aqui.
     /// Perfil e servidor. As duas telas só descrevem o que querem.
     /// Sobre: nome, versão e a que servidor a janela está ligada.
     /// O mesmo formulário, aplicado ao estado local da demonstração.
    /// Traduz o que a folha pediu do lado da conta e do servidor.
    fn handle_admin(&mut self, ctx: &egui::Context, action: crate::ui::admin::AdminAction) {
        use crate::ui::admin::AdminAction;

        // Os dois seletores de imagem passam pelo portal do sistema, que
        // responde noutro quadro.
        let command = match action {
            AdminAction::PickAvatar => {
                self.dialogs.pick_image(ctx.clone(), ImagePick::Avatar);
                return;
            }
            AdminAction::PickSticker => {
                self.dialogs.pick_image(ctx.clone(), ImagePick::Sticker);
                return;
            }
            AdminAction::SaveProfile(request) => Command::UpdateProfile(request),
            AdminAction::SetPresence(status) => Command::SetStatus { status },
            AdminAction::ChangePassword(password) => Command::ChangePassword { password },
            AdminAction::LoadDevices => Command::LoadDevices,
            AdminAction::DropConnection(connection_id) => Command::DropConnection { connection_id },
            AdminAction::SaveServer(request) => Command::UpdateServer(request),
            AdminAction::CreateSticker { name, blob, format } => {
                Command::CreateEmoji { name, blob, format }
            }
            AdminAction::DeleteEmoji(emoji_id) => Command::DeleteEmoji { emoji_id },
            AdminAction::LoadAuditLogs => Command::LoadAuditLogs,
        };
        self.workspaces[self.active].net.send(command);
    }

    fn handle_role(&mut self, action: crate::ui::roles::RoleAction) {
        use crate::ui::roles::RoleAction;

        let command = match action {
            RoleAction::Create {
                name,
                color,
                permissions,
            } => Command::CreateRole {
                name,
                color,
                permissions,
            },
            RoleAction::Update {
                role_id,
                name,
                color,
                permissions,
            } => Command::UpdateRole {
                role_id,
                name,
                color,
                permissions,
            },
            RoleAction::Delete(role_id) => Command::DeleteRole { role_id },
            RoleAction::Assign { user_id, role_id } => Command::AssignRole { user_id, role_id },
            RoleAction::Unassign { user_id, role_id } => Command::UnassignRole { user_id, role_id },
        };
        self.workspaces[self.active].net.send(command);
    }

    /// A folha de ajustes, ancorada na pastilha que a abriu. É a única
    /// tela de ajustes que existe: conta, aparência, avisos, arquivos,
    /// idioma e sessões de um lado; servidor, canais, cargos, figurinhas e
    /// auditoria do outro.
    fn settings_sheet(&mut self, ctx: &egui::Context) {
        use crate::ui::settings::{SettingsAction, Surface};

        let Some(surface) = self.sheet.open else {
            return;
        };
        let s = self.settings.lang.strings();
        let t = self.tokens;
        let screen = ctx.content_rect();
        // A folha nasce exatamente na pastilha que a abriu — mesma borda
        // esquerda, colada na de cima ou na de baixo conforme o caso. Antes
        // eram números soltos, e a folha saía uns pixels fora da pastilha.
        use crate::ui::shell::{IDENTITY_PILL_HEIGHT, PILL_INSET, SIDEBAR_WIDTH};
        let left = screen.min.x + crate::ui::rail::RAIL_WIDTH + PILL_INSET;
        let width = SIDEBAR_WIDTH - PILL_INSET * 2.0;
        let anchor = match surface {
            Surface::Server => egui::Rect::from_min_size(
                egui::pos2(left, screen.min.y + PILL_INSET),
                egui::vec2(width, IDENTITY_PILL_HEIGHT),
            ),
            Surface::App => egui::Rect::from_min_size(
                egui::pos2(
                    left,
                    screen.max.y - IDENTITY_PILL_HEIGHT - PILL_INSET,
                ),
                egui::vec2(width, IDENTITY_PILL_HEIGHT),
            ),
        };

        let mut ask_download = self.settings.downloads == DownloadMode::Ask;
        // O início automático é estado do sistema, não um ajuste guardado: o
        // que vale é o arquivo em disco, então ele é lido e escrito à parte.
        let autostart_before = crate::platform::autostart::is_enabled();
        let mut autostart = autostart_before;
        let before = (
            self.settings.lang,
            self.settings.theme,
            self.settings.translucency,
            self.settings.show_members,
            self.settings.notifications,
            self.settings.reply_notifications,
            self.settings.close_to_tray,
            self.settings.badge,
            self.settings.topic_reveal,
            self.settings.record_button,
            ask_download,
        );

        let diagnostics: Vec<crate::ui::settings::WorkspaceDiagnostics> = self
            .workspaces
            .iter()
            .map(|workspace| crate::ui::settings::WorkspaceDiagnostics {
                label: workspace.label.clone(),
                server_key: crate::state::server_key(&workspace.url),
                runtime: workspace.net.diagnostics(),
                store: workspace.store.diagnostics(),
                cache_enabled: workspace.cache.is_enabled(),
                cache: workspace.cache.stats(),
            })
            .collect();

        let actions = {
            let ws = &self.workspaces[self.active];
            let mut data = crate::ui::settings::Context {
                store: &ws.store,
                media: &mut self.ui.media,
                roles: &mut self.roles,
                lang: &mut self.settings.lang,
                theme: &mut self.settings.theme,
                translucency: &mut self.settings.translucency,
                notifications: &mut self.settings.notifications,
                reply_notifications: &mut self.settings.reply_notifications,
                close_to_tray: &mut self.settings.close_to_tray,
                autostart: &mut autostart,
                badge: &mut self.settings.badge,
                topic_reveal: &mut self.settings.topic_reveal,
                record_button: &mut self.settings.record_button,
                ask_download: &mut ask_download,
                download_dir: match &self.settings.downloads {
                    DownloadMode::Folder(dir) => Some(dir.display().to_string()),
                    DownloadMode::Ask => None,
                },
                diagnostics: &diagnostics,
            };
            crate::ui::settings::sheet(ctx, &mut self.sheet, &mut data, anchor, screen, &t, s)
        };

        // Um ajuste local mudou: grava e reflete na janela na hora.
        let after = (
            self.settings.lang,
            self.settings.theme,
            self.settings.translucency,
            self.settings.show_members,
            self.settings.notifications,
            self.settings.reply_notifications,
            self.settings.close_to_tray,
            self.settings.badge,
            self.settings.topic_reveal,
            self.settings.record_button,
            ask_download,
        );
        if before != after {
            if ask_download != before.10 {
                self.settings.downloads = if ask_download {
                    DownloadMode::Ask
                } else {
                    DownloadMode::Folder(files::downloads_dir())
                };
            }
            self.retheme(ctx);
        }

        if autostart != autostart_before
            && let Err(error) = crate::platform::autostart::set(autostart)
        {
            log::warn!("não deu para ajustar o início automático: {error}");
        }

        for action in actions {
            match action {
                SettingsAction::Menu(command) => self.ui.pending.push(command),
                SettingsAction::PickDownloadFolder => {
                    let start = match &self.settings.downloads {
                        DownloadMode::Folder(dir) => dir.clone(),
                        DownloadMode::Ask => files::downloads_dir(),
                    };
                    self.dialogs.pick_folder(ctx.clone(), start);
                }
                SettingsAction::Chat(chat) => self.handle_chat(ctx, chat),
                SettingsAction::Admin(admin) => self.handle_admin(ctx, admin),
                SettingsAction::Role(role) => self.handle_role(role),
            }
        }
    }


 }

#[cfg(target_os = "android")]
impl PapoApp {
    fn sync_android_notification_contexts(&self) {
        for (index, workspace) in self.workspaces.iter().enumerate() {
            workspace.sync_notification_context(
                self.settings.notifications,
                index == self.active,
            );
        }
    }
}

impl eframe::App for PapoApp {
    /// Tira da área desenhável as bordas que o sistema ocupa.
    ///
    /// É o único ajuste de Android na interface, e de propósito ele entra
    /// aqui: encolhendo o `screen_rect` antes de o egui ver o quadro, todo o
    /// resto — painéis, rolagem, folhas — continua o mesmo dos dois lados.
    /// Nada na interface precisa saber que existe uma barra de status.
    #[cfg(target_os = "android")]
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let Some(rect) = raw_input.screen_rect else {
            return;
        };
        // As bordas vêm em pixels físicos; o `screen_rect` é em pontos.
        crate::platform::wake::install(ctx);
        let scale = ctx.pixels_per_point().max(0.1);
        let (left, top, right, bottom) = crate::platform::safe_area::insets_px();
        let safe = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + left / scale, rect.min.y + top / scale),
            egui::pos2(rect.max.x - right / scale, rect.max.y - bottom / scale),
        );
        // Um aparelho que informe bordas maiores que a própria tela não pode
        // apagar a janela inteira.
        if safe.width() > 1.0 && safe.height() > 1.0 {
            raw_input.screen_rect = Some(safe);
        }

        // O winit sobe o teclado mas não entrega o que se digita nele; quem
        // faz essa parte é a ponte.
        crate::platform::ime::pump(ctx, raw_input);
    }

    /// O eframe grava sozinho de trinta em trinta segundos e, fora isso, ao
    /// encerrar limpo. No Android não existe encerrar limpo: o sistema mata
    /// o processo quando quiser, e com ele ia embora tudo o que se fez desde
    /// a última gravação — um servidor recém-adicionado, por exemplo.
    #[cfg(target_os = "android")]
    fn auto_save_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(5)
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        #[cfg(target_os = "android")]
        {
            crate::platform::native_text::begin_frame();
            crate::platform::native_field::begin_frame();
            if self.settings.notifications {
                crate::platform::android_message::ensure_permission();
            }
        }
        self.attach_window(frame);

        #[cfg(target_os = "android")]
        {
            self.handle_android_notification_navigation(&ctx);
            self.sync_android_notification_contexts();
        }

        // Indo para segundo plano: gravar agora, porque pode não haver um
        // depois. Perder o foco é o último aviso que o aplicativo recebe
        // antes de o sistema poder encerrá-lo sem mais nada.
        #[cfg(target_os = "android")]
        {
            let focused = ctx.input(|input| input.viewport().focused).unwrap_or(true);
            if self.focused && !focused {
                if let Some(storage) = frame.storage_mut() {
                    eframe::App::save(self, storage);
                    storage.flush();
                    log::debug!("ajustes gravados ao sair de cena");
                }
            }
            self.focused = focused;
        }
        #[cfg(target_os = "linux")]
        {
            self.sync_menu();
            self.sync_blur_regions(&ctx);
        }
        #[cfg(target_os = "linux")]
        self.handle_window_lifecycle(&ctx);

        self.pump_network(&ctx);
        #[cfg(target_os = "android")]
        self.sync_android_notification_contexts();

        // A call segue viva com outro servidor na tela: o trilho troca a
        // conversa, não quem está falando.
        for ws in &mut self.workspaces {
            pump_call(ws);
        }

        #[cfg(target_os = "android")]
        {
            for action in crate::platform::android_call::take_actions() {
                match action {
                    crate::platform::android_call::UiAction::Muted(muted) => {
                        if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.store.call.active()) {
                            ws.store.call.muted = muted;
                        }
                    }
                    crate::platform::android_call::UiAction::Camera(camera) => {
                        if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.store.call.active()) {
                            ws.store.call.camera = camera;
                        }
                    }
                    crate::platform::android_call::UiAction::Hangup => {
                        if let Some(index) = self.workspaces.iter().position(|ws| ws.store.call.active()) {
                            leave_call(&mut self.workspaces[index]);
                        }
                    }
                }
            }

            let call_index = self.workspaces.iter().position(|ws| ws.store.call.active());
            let has_video = call_index
                .is_some_and(|index| self.workspaces[index].store.call.has_video());

            let (muted, camera, members, speaker_name) = if let Some(index) = call_index {
                let ws = &self.workspaces[index];
                let speaker_name = ws.store.call.speakers.first().and_then(|id| {
                    ws.store.member(id).map(|member| member.name.clone())
                });
                (
                    ws.store.call.muted,
                    ws.store.call.camera,
                    ws.store.call.members().len(),
                    speaker_name,
                )
            } else {
                (true, false, 0, None)
            };

            if call_index.is_some() {
                crate::platform::android_call::sync_service(
                    muted,
                    camera,
                    members,
                    speaker_name.as_deref(),
                );
            }
            crate::platform::android_call::set_presentation(
                call_index.is_some(),
                has_video,
                muted,
                camera,
            );

            if crate::platform::android_call::is_in_pip() {
                ctx.request_repaint();
                if let Some(index) = call_index {
                    let strings = self.settings.lang.strings();
                    let ws = &mut self.workspaces[index];
                    crate::ui::call::pip(
                        ui,
                        &ws.store,
                        &mut self.ui,
                        ws.call.as_mut(),
                        &self.tokens,
                        strings,
                    );
                }
                crate::platform::native_field::end_frame();
                crate::platform::native_text::end_frame();
                return;
            }
        }

        let strings = self.settings.lang.strings();
        self.draw_header(ui);

        let compact_chat = matches!(
            self.workspaces[self.active].store.screen,
            Screen::Chat
        ) && crate::ui::shell::is_compact(ctx.content_rect());
        let add_server_modal = self.add_server_previous.is_some();
        if !compact_chat && !add_server_modal {
            self.draw_rail(ui, strings, &ctx);
        }

        let active = self.active;
        match self.workspaces[active].store.screen {
            Screen::Starting => auth::starting(ui, &self.tokens, strings),
            Screen::Auth => {
                let response = {
                    let ws = &mut self.workspaces[active];
                    auth::sign_in(ui, &mut ws.form, &ws.store, &self.tokens, strings)
                };
                match response.action {
                    AuthAction::SignIn => self.authenticate(false, &ctx),
                    AuthAction::Register => self.authenticate(true, &ctx),
                    AuthAction::UnlockServer => self.unlock_server(),
                    _ => {}
                }
                if add_server_modal && Self::clicked_outside_auth_card(&ctx, response.rect) {
                    self.cancel_add_server(&ctx);
                }
            }
            Screen::NeedsServer => {
                let response = {
                    let ws = &mut self.workspaces[active];
                    auth::create_server(ui, &mut ws.form, &ws.store, &self.tokens, strings)
                };
                if response.action == AuthAction::CreateServer {
                    let ws = &mut self.workspaces[active];
                    ws.store.busy = true;
                    ws.net.send(Command::CreateServer {
                        name: ws.form.server_name.trim().to_owned(),
                    });
                }
                if add_server_modal && Self::clicked_outside_auth_card(&ctx, response.rect) {
                    self.cancel_add_server(&ctx);
                }
            }
            Screen::Chat => {
                // A autenticação terminou; a partir daqui o servidor deixa de
                // ser provisório e passa a fazer parte do trilho normalmente.
                self.add_server_previous = None;
                let mobile_entries = compact_chat.then(|| {
                    self.workspaces
                        .iter()
                        .map(|ws| ws.entry(&self.settings))
                        .collect::<Vec<_>>()
                });
                self.ui.reply_notify_default = self.settings.reply_notifications;
                let rail_action = {
                    let ws = &mut self.workspaces[active];
                    shell::draw(
                        ui,
                        &mut ws.store,
                        &mut self.ui,
                        ws.call.as_mut(),
                        &self.tokens,
                        strings,
                        mobile_entries.as_ref().map(|entries| shell::MobileServers {
                            entries,
                            active,
                        }),
                    )
                };
                if let Some(action) = rail_action {
                    self.handle_rail_action(action, &ctx);
                }
                self.call_window(&ctx);
                self.pump_chat();
                let actions = std::mem::take(&mut self.ui.actions);
                for action in actions {
                    self.handle_chat(&ctx, action);
                }
            }
        }
        self.settings_sheet(&ctx);
        self.pump_files(&ctx);

        if self.own_chrome {
            crate::ui::headerbar::resize_handles(&ctx);
        }

        let pending = std::mem::take(&mut self.ui.pending);
        for command in pending {
            self.handle(&ctx, command);
        }

        #[cfg(target_os = "android")]
        {
            crate::platform::native_field::end_frame();
            crate::platform::native_text::end_frame();
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let (Some(gl), Some(glass)) = (gl, &self.ui.glass)
            && let Ok(glass) = glass.lock()
        {
            glass.destroy(gl);
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // As marcas de leitura vivem no estado de cada servidor; só o ajuste
        // persiste, e cada servidor guarda as suas sob a própria chave.
        let draft = self.add_server_previous.map(|_| self.active);
        for (index, ws) in self.workspaces.iter().enumerate() {
            if Some(index) == draft {
                continue;
            }
            self.settings
                .server_marks
                .insert(crate::state::server_key(&ws.url), ws.store.read_marks.clone());
        }

        // O rascunho do botão + é estado de UI, não um servidor. Em especial
        // no Android o autosave roda enquanto o cartão está aberto e o
        // processo pode morrer logo depois; persistir o rascunho transformava
        // um simples + em outro servidor na próxima abertura.
        let mut persisted = self.settings.clone();
        persisted.servers = self
            .workspaces
            .iter()
            .enumerate()
            .filter(|(index, _)| Some(*index) != draft)
            .map(|(_, ws)| ServerEntry {
                url: ws.url.clone(),
                label: ws.label.clone(),
            })
            .collect();
        persisted.active = self.add_server_previous.unwrap_or(self.active);
        persisted.active = persisted.active.min(persisted.servers.len().saturating_sub(1));
        if let Some(entry) = persisted.servers.get(persisted.active) {
            persisted.server_url = entry.url.clone();
        }
        eframe::set_value(storage, eframe::APP_KEY, &persisted);
    }

    /// A janela é opaca: o vidro é desenhado por nós, não pelo compositor.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let c = self.tokens.content_bg;
        [
            c.r() as f32 / 255.0,
            c.g() as f32 / 255.0,
            c.b() as f32 / 255.0,
            1.0,
        ]
    }
}

fn appearance_for(settings: &Settings, system: &SystemTheme) -> Appearance {
    match settings.theme {
        ThemePref::Light => Appearance::Light,
        ThemePref::Dark => Appearance::Dark,
        ThemePref::System => {
            if system.prefers_dark.unwrap_or(true) {
                Appearance::Dark
            } else {
                Appearance::Light
            }
        }
    }
}

/// Árvore do menu global, montada a partir do estado atual.
fn build_menu(settings: &Settings) -> MenuModel {
    let s = settings.lang.strings();
    MenuModel::new(vec![
        MenuNode::submenu(
            s.menu_papo,
            vec![
                MenuNode::item(s.menu_about, MenuCommand::About),
                MenuNode::separator(),
                MenuNode::item(s.menu_preferences, MenuCommand::Preferences)
                    .accel(&["Control", "comma"]),
                MenuNode::item(s.menu_roles, MenuCommand::Roles),
                MenuNode::item(s.menu_server, MenuCommand::ServerSettings),
                MenuNode::separator(),
                MenuNode::item(s.sign_out, MenuCommand::SignOut),
                MenuNode::separator(),
                MenuNode::item(s.menu_quit, MenuCommand::Quit).accel(&["Control", "q"]),
            ],
        ),
        MenuNode::submenu(
            s.menu_file,
            vec![
                MenuNode::item(s.menu_new_channel, MenuCommand::NewChannel)
                    .accel(&["Control", "n"]),
                MenuNode::item(s.menu_search, MenuCommand::Search).accel(&["Control", "f"]),
                MenuNode::separator(),
                MenuNode::item(s.menu_mark_all_read, MenuCommand::MarkAllRead),
            ],
        ),
        MenuNode::submenu(
            s.menu_view,
            vec![
                MenuNode::checkbox(
                    s.menu_toggle_members,
                    MenuCommand::ToggleMembers,
                    settings.show_members,
                )
                .accel(&["Control", "u"]),
                MenuNode::checkbox(
                    s.translucency,
                    MenuCommand::ToggleTranslucency,
                    settings.translucency,
                ),
                MenuNode::separator(),
                MenuNode::checkbox(
                    s.menu_notifications,
                    MenuCommand::ToggleNotifications,
                    settings.notifications,
                ),
                MenuNode::checkbox(
                    s.menu_close_to_tray,
                    MenuCommand::ToggleCloseToTray,
                    settings.close_to_tray,
                ),
                MenuNode::checkbox(s.badge, MenuCommand::ToggleBadge, settings.badge),
                MenuNode::checkbox(
                    s.topic_reveal,
                    MenuCommand::ToggleTopicReveal,
                    settings.topic_reveal,
                ),
                MenuNode::checkbox(
                    s.record_button,
                    MenuCommand::ToggleRecordButton,
                    settings.record_button,
                ),
                MenuNode::separator(),
                MenuNode::submenu(
                    s.menu_language,
                    Lang::ALL
                        .iter()
                        .map(|lang| {
                            MenuNode::radio(
                                lang.endonym(),
                                MenuCommand::SwitchLanguage(*lang),
                                settings.lang == *lang,
                            )
                        })
                        .collect(),
                ),
            ],
        ),
        MenuNode::submenu(
            s.menu_help,
            vec![MenuNode::item(s.menu_about, MenuCommand::About)],
        ),
    ])
}

/// Dá um esquema ao endereço quando ele vem sem. `192.168.0.114:8080` vira
/// `http://…`, que é o que um servidor de casa espera; um domínio público
/// vira `https://…`. Sem isto o `Url::parse` recusa, a conexão morre no
/// nascimento e a tela fica presa em "Conectando…".
fn normalise_server_url(input: &str) -> String {
    let url = input.trim();
    if url.is_empty() || url.contains("://") {
        return url.to_owned();
    }
    let host = host_part(url);
    let scheme = if is_local_host(&host) { "http" } else { "https" };
    format!("{scheme}://{url}")
}

/// O host de um endereço sem esquema, para decidir o esquema.
fn host_part(url: &str) -> String {
    if let Some(rest) = url.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest).to_owned();
    }
    url.split(['/', ':']).next().unwrap_or(url).to_owned()
}

/// Endereços que só existem na rede local ou na própria máquina.
fn is_local_host(host: &str) -> bool {
    if matches!(host, "localhost" | "127.0.0.1" | "::1" | "0.0.0.0") {
        return true;
    }
    if host.starts_with("10.") || host.starts_with("192.168.") || host.starts_with("169.254.") {
        return true;
    }
    if let Some(rest) = host.strip_prefix("172.")
        && let Some(octet) = rest
            .split('.')
            .next()
            .and_then(|part| part.parse::<u8>().ok())
    {
        return (16..=31).contains(&octet);
    }
    false
}

/// Nome curto de um endereço: só o host, que é o que cabe no trilho.
fn host_of(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_owned))
        .unwrap_or_else(|| url.trim().trim_end_matches('/').to_owned())
}

fn default_server_url() -> String {
    "https://papo-backend.onrender.com".to_owned()
}

#[cfg(target_os = "linux")]
fn tray_labels(settings: &Settings) -> TrayLabels {
    let s = settings.lang.strings();
    TrayLabels {
        open: s.tray_open.to_owned(),
        quit: s.tray_quit.to_owned(),
        tooltip: s.tray_tooltip.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::normalise_server_url;

    /// Endereço sem esquema é o caso comum de um servidor de casa; ele tem de
    /// virar uma URL que o `reqwest` aceite, ou a conexão morre em silêncio.
    #[test]
    fn endereco_sem_esquema_recebe_um() {
        assert_eq!(
            normalise_server_url("192.168.0.114:8080"),
            "http://192.168.0.114:8080"
        );
        assert_eq!(normalise_server_url("localhost:3000"), "http://localhost:3000");
        assert_eq!(normalise_server_url("10.0.0.5"), "http://10.0.0.5");
        assert_eq!(normalise_server_url("172.16.1.2:9000"), "http://172.16.1.2:9000");
        assert_eq!(
            normalise_server_url("papo.exemplo.com"),
            "https://papo.exemplo.com"
        );
    }

    #[test]
    fn endereco_com_esquema_fica_como_esta() {
        assert_eq!(
            normalise_server_url("https://papo-backend.onrender.com"),
            "https://papo-backend.onrender.com"
        );
        assert_eq!(normalise_server_url("  http://x  "), "http://x");
        assert_eq!(normalise_server_url(""), "");
    }
}