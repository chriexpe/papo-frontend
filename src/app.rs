//! Aplicação: junta estado, tema e telas.

use egui::Color32;

use crate::i18n::Lang;
use crate::media::Media;
use crate::platform::files::{self, Chosen, Dialogs, ImagePick};
use crate::platform::menu::MenuCommand;
#[cfg(target_os = "linux")]
use crate::platform::menu::{MenuModel, MenuNode};
use crate::platform::desktop;
#[cfg(target_os = "linux")]
use crate::platform::activate::Activator;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::platform::notify::{Notification, Notifier};
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::platform::tray::{Tray, TrayCommand, TrayLabels};
#[cfg(target_os = "linux")]
use crate::platform::launcher::{Badge, Launcher};
#[cfg(target_os = "linux")]
use crate::platform::{appmenu::AppMenuSurface, blur::BlurSurface, global_menu::GlobalMenu};
use crate::api::net::{Command, Wake};
use crate::state::{Activity, Phase, Screen, Store};
use papo_core::cache::{CachedMessagePage, ClientDb};
use papo_core::notification::{NotificationCoordinator, NotificationSink};
use papo_core::runtime::{
    CallRuntimeEffect, RuntimeEffect, RuntimeNotificationView, ServerRuntime,
};
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
fn route_call_effect(ws: &mut Workspace, effect: CallRuntimeEffect, ctx: &egui::Context) {
    use crate::api::ws::Event;

    match effect {
        CallRuntimeEffect::VoiceReady {
            channel_id,
            attempt,
            servers,
        } => {
            if !ws.runtime.store.call.current(&channel_id, attempt) {
                return;
            }
            ws.call = Call::start(
                channel_id.clone(),
                IceConfig::from_servers(&servers),
                ctx.clone(),
                ws.runtime.net.sender(),
            );
            ws.call_ready = false;
            if let Some(call) = &ws.call {
                let names = ws
                    .runtime
                    .store
                    .members
                    .iter()
                    .map(|member| (member.id.clone(), member.name.clone()))
                    .collect();
                ws.runtime.net.set_event_callback(Some(call.event_callback(
                    ws.runtime.store.me.clone(),
                    names,
                )));
                #[cfg(target_os = "android")]
                crate::platform::android_call::bind(
                    call.command_sender(),
                    ws.runtime.net.sender(),
                    channel_id,
                    ws.runtime.store.call.muted,
                    ws.runtime.store.call.camera,
                    ws.runtime.store.me.clone(),
                );
            } else {
                ws.runtime.store.call.error = Some("a call não abriu".to_owned());
                ws.runtime.store.call.left();
                #[cfg(target_os = "android")]
                crate::platform::android_call::stop_service();
            }
        }
        CallRuntimeEffect::Event {
            event,
            me_before,
            phase_before,
            store_error_before,
            call_error_before,
        } => {
            let Some(call) = &ws.call else { return };
            let mut over = false;
            let mut tell_server = false;
            match *event {
                Event::VoiceLeft {
                    channel_id,
                    user_id,
                } if channel_id == call.channel_id && user_id == me_before => over = true,
                Event::Failure { code, .. }
                    if code.as_deref().is_some_and(|code| {
                        crate::state::call::fatal(code, phase_before == Phase::Joining)
                    }) =>
                {
                    over = true;
                    tell_server = phase_before == Phase::In;
                }
                _ => {}
            }
            if over {
                if tell_server {
                    let channel_id = call.channel_id.clone();
                    ws.runtime.net.send(Command::VoiceSignal(format!(
                        r#"{{"type":"voice_leave","channel_id":"{}"}}"#,
                        channel_id
                    )));
                    // Before the runtime split, this teardown happened before
                    // Store::apply. Restore the pre-update error projection so
                    // a structural refactor does not change call UX.
                    ws.runtime.store.error = store_error_before;
                    ws.runtime.store.call.error = call_error_before;
                    ws.camera_revision = 0;
                }
                ws.runtime.net.set_event_callback(None);
                ws.call = None;
                ws.call_ready = false;
                ws.watching.clear();
                #[cfg(target_os = "android")]
                crate::platform::android_call::stop_service();
            }
        }
        CallRuntimeEffect::ConnectionOffline if ws.runtime.store.call.active() => {
            ws.runtime.net.set_event_callback(None);
            ws.call = None;
            ws.call_ready = false;
            ws.watching.clear();
            ws.runtime.store.call.error = None;
            ws.runtime.store.call.left();
            #[cfg(target_os = "android")]
            crate::platform::android_call::stop_service();
        }
        CallRuntimeEffect::VoiceFailed { was_current: true, .. } => {
            #[cfg(target_os = "android")]
            crate::platform::android_call::stop_service();
        }
        _ => {}
    }
}

/// Sai da call: avisa o servidor, desmonta o pipeline e limpa o retrato.
fn leave_call(ws: &mut Workspace) {
    #[cfg(target_os = "android")]
    let had_call = ws.runtime.store.call.active() || ws.call.is_some();
    if ws.runtime.store.call.active() && !ws.runtime.store.call.channel_id.is_empty() {
        ws.runtime.net.send(Command::VoiceSignal(format!(
            r#"{{"type":"voice_leave","channel_id":"{}"}}"#,
            ws.runtime.store.call.channel_id
        )));
    }
    ws.runtime.net.set_event_callback(None);
    ws.call = None;
    ws.call_ready = false;
    ws.watching.clear();
    ws.camera_revision = 0;
    ws.runtime.store.call.left();
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
    if !ws.call_ready && ws.runtime.store.call.phase == Phase::In {
        call.ready();
        ws.call_ready = true;
        // O que foi clicado enquanto a call abria vale: entramos mudos, como
        // o servidor assume, mas quem já tinha ligado a câmera não pode ficar
        // com o botão aceso e a câmera parada.
        call.set_muted(ws.runtime.store.call.muted);
        if ws.runtime.store.call.camera {
            call.set_camera(true);
        }
    }

    // Aviso que não derruba a call (sem microfone, sem câmera): aparece uma
    // vez, como os outros recados passageiros da tela.
    if let Some(warning) = call.take_warning() {
        ws.runtime.store.error = Some(warning);
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
        ws.runtime.store.call.camera = camera;
    }

    if call.failed() {
        let message = call.error();
        ws.runtime.store.call.error = message.clone();
        // O mesmo aviso passageiro dos outros erros: quem está na tela
        // precisa saber por que a call sumiu.
        ws.runtime.store.error = message;
        leave_call(ws);
        return;
    }

    // Quem ligou a câmera passa a ser assistido sozinho — a escolha de
    // desenho. Daqui sai só a lista de quem se quer ver; quem decide o que
    // cabe nos seis lugares é a thread da call, que é quem sabe quais estão
    // livres e reaproveita o que vaga.
    let me = ws.runtime.store.me.clone();
    let mut wanted: Vec<String> = ws.runtime
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
    /// Ao abrir um canal com não lidas, começa no head e preserva o bloco
    /// pulado como checkpoint de retorno.
    #[serde(default = "enabled")]
    pub open_at_newest: bool,
    /// Botão de gravar recado ao lado da caixa de texto.
    #[serde(default = "enabled")]
    pub record_button: bool,
    /// Emoji dentro do nome de canal segue a cor do rótulo em vez das cores
    /// originais. É só apresentação local: o nome salvo não muda.
    #[serde(default = "enabled")]
    pub channel_emoji_monochrome: bool,
    /// Como o seu cartão de perfil abre: da pastilha ou flutuante.
    #[serde(default)]
    pub self_card: crate::ui::profile::SelfCardStyle,
    /// Ao rolar um WebEmbed ativo para fora da timeline: encerrar ou flutuar.
    #[serde(default)]
    pub webembed_offscreen: crate::webembed::OffscreenBehavior,
    /// Se flutuando, limita ao canal atual ou acompanha toda a navegação.
    #[serde(default)]
    pub webembed_scope: crate::webembed::FloatScope,
    /// Largura em pontos do player flutuante no desktop.
    #[serde(default = "default_webembed_float_width")]
    pub webembed_float_width: f32,
    /// Canto superior esquerdo do player flutuante, em pontos. `None` usa o
    /// canto inferior padrão; um arrasto grava a posição escolhida.
    #[serde(default)]
    pub webembed_float_pos: Option<(f32, f32)>,
    /// HTTPS hosts the user explicitly chose to open without another prompt.
    #[serde(default)]
    pub trusted_link_hosts: std::collections::BTreeSet<String>,
    #[serde(default)]
    pub downloads: DownloadMode,
    /// Stable GIPHY IDs only; provider media stays transient.
    /// A provider-specific persisted key intentionally avoids importing old
    /// KLIPY favourite identifiers as if they were GIPHY IDs.
    #[serde(default, rename = "giphy_favourites")]
    pub gif_favourites: std::collections::BTreeSet<String>,
    /// Rich Presence deste dispositivo. Não é sincronizado com a conta:
    /// processo local, bridge arRPC e override são propriedades da máquina.
    #[serde(default)]
    pub rich_presence: crate::rich_presence::Settings,
    /// Marcas de leitura de quando havia um servidor só; migradas na
    /// primeira abertura e depois vazias.
    #[serde(default)]
    pub read_marks: std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
    /// Até quando cada canal foi visto, por servidor. É o que sobrevive ao
    /// fechamento, já que o backend registra `last_read_message` mas nunca o
    /// escreve.
    #[serde(default)]
    pub server_marks: ReadMarks,
    /// Ajustes portáteis alterados localmente e ainda não confirmados pelo
    /// servidor. A chave é o server_key; persiste para sobreviver offline.
    #[serde(default)]
    pub pending_user_settings:
        std::collections::HashMap<String, crate::api::models::UserConfig>,
    /// Último snapshot remoto conhecido por servidor. Serve para startup e
    /// troca offline; nunca vence um pending local nem um whoami fresco.
    #[serde(default)]
    pub cached_user_settings:
        std::collections::HashMap<String, crate::api::models::UserConfig>,
    /// Servidores que aceitaram o token de push deste aparelho: server_key →
    /// token. Nestes o FCM entrega as notificações em segundo plano e a
    /// reconciliação periódica fica desligada. Só o Android escreve aqui.
    #[serde(default)]
    pub push_devices: std::collections::HashMap<String, String>,
    /// Versão cuja oferta foi dispensada. Enquanto ela continuar sendo a
    /// release atual, o app mostra só o affordance de download no chrome em
    /// vez de reabrir o banner a cada inicialização.
    #[serde(default)]
    pub dismissed_update_version: Option<String>,
}

/// Marcas de leitura por servidor: chave do servidor → canal → instante.
pub type ReadMarks = std::collections::HashMap<
    String,
    std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
>;

fn enabled() -> bool {
    true
}

fn default_webembed_float_width() -> f32 {
    360.0
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
            open_at_newest: true,
            record_button: true,
            channel_emoji_monochrome: true,
            self_card: crate::ui::profile::SelfCardStyle::default(),
            webembed_offscreen: crate::webembed::OffscreenBehavior::default(),
            webembed_scope: crate::webembed::FloatScope::default(),
            webembed_float_width: default_webembed_float_width(),
            webembed_float_pos: None,
            trusted_link_hosts: std::collections::BTreeSet::new(),
            downloads: DownloadMode::default(),
            gif_favourites: std::collections::BTreeSet::new(),
            rich_presence: crate::rich_presence::Settings::default(),
            read_marks: std::collections::HashMap::new(),
            server_marks: ReadMarks::new(),
            pending_user_settings: std::collections::HashMap::new(),
            cached_user_settings: std::collections::HashMap::new(),
            push_devices: std::collections::HashMap::new(),
            dismissed_update_version: None,
        }
    }
}

impl Settings {
    /// Garante que a lista de servidores existe e que o ativo aponta para um
    /// item de verdade. Ajustes gravados antes do trilho só têm `server_url`.
    fn normalise(&mut self) {
        self.server_url = normalise_server_url(&self.server_url);
        self.gif_favourites.retain(|slug| crate::giphy::valid_id(slug));
        self.trusted_link_hosts = std::mem::take(&mut self.trusted_link_hosts)
            .into_iter()
            .map(|host| host.trim_end_matches('.').to_ascii_lowercase())
            .filter(|host| !host.is_empty())
            .collect();

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
        let known: std::collections::HashSet<String> = self
            .servers
            .iter()
            .map(|entry| crate::state::server_key(&entry.url))
            .collect();
        self.push_devices.retain(|key, _| known.contains(key));

        // Antes de criar tokens de tema / pedir permissão nativa, projeta o
        // último config conhecido da conta ativa. Pending local tem prioridade.
        let active_key = crate::state::server_key(&self.server_url);
        if let Some(config) = self
            .pending_user_settings
            .get(&active_key)
            .or_else(|| self.cached_user_settings.get(&active_key))
        {
            self.theme = match config.theme.as_str() {
                "dark" => ThemePref::Dark,
                "light" => ThemePref::Light,
                _ => ThemePref::System,
            };
            self.notifications = config.notifications.enabled;
        }
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

struct PendingCachePage {
    channel_id: String,
    older: bool,
    restore_epoch: u64,
    receiver: std::sync::mpsc::Receiver<Result<CachedMessagePage, String>>,
}

/// Um servidor conectado: rede, estado e o que a interface guarda dele.
///
/// Todos ficam ligados ao mesmo tempo — é o que faz a menção de um servidor
/// que não está na tela ainda acender o contador da bandeja.
pub struct Workspace {
    pub runtime: ServerRuntime,
    pub label: String,
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
    /// Última atividade local entregue ao runtime deste servidor. O Option
    /// externo distingue "ainda não publicamos" de "publicamos sem atividade".
    published_activity: Option<Option<Activity>>,
    /// Último sinal de interação real enviado para o auto-away do backend.
    last_presence_activity: Option<std::time::Instant>,
    /// Último config remoto aplicado à UI enquanto este servidor estava ativo.
    applied_user_config: Option<crate::api::models::UserConfig>,
    /// Config já enviado nesta conexão; evita PUT a cada frame.
    sent_user_config: Option<crate::api::models::UserConfig>,
    /// Leituras Turso em voo. O worker de cache faz SQL; egui só sonda o receiver.
    cache_pages: Vec<PendingCachePage>,
    /// Último pedido de push feito nesta conexão: (token, registrar?). O
    /// pedido só sai de novo quando um dos dois muda.
    #[cfg(target_os = "android")]
    push_sent: Option<(String, bool)>,
    #[cfg(target_os = "android")]
    _network_registration: crate::platform::android_network::Registration,
}

impl Workspace {
    fn open(
        entry: &ServerEntry,
        marks: &ReadMarks,
        ctx: &egui::Context,
        cache: &std::sync::Arc<ClientDb>,
        notification: &std::sync::Arc<NotificationCoordinator>,
    ) -> Self {
        let server_key = crate::state::server_key(&entry.url);
        let mut store = Store::default();
        store.read_marks = marks.get(&server_key).cloned().unwrap_or_default();

        let repaint = ctx.clone();
        let runtime = ServerRuntime::open_with_store(
            entry.url.clone(),
            store,
            Wake::new(move || repaint.request_repaint()),
            std::sync::Arc::new(crate::storage::FileSecretStore::new()),
            std::sync::Arc::clone(cache),
            std::sync::Arc::clone(notification),
        );

        #[cfg(target_os = "android")]
        let network_registration =
            crate::platform::android_network::register(runtime.sender());

        // A mídia usa o cookie da sessão deste servidor para baixar anexos.
        let media = Media::spawn(
            entry.url.clone(),
            std::sync::Arc::clone(&runtime.net.session),
            ctx.clone(),
        );
        Self {
            runtime,
            label: entry.label.clone(),
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
            published_activity: None,
            last_presence_activity: None,
            applied_user_config: None,
            sent_user_config: None,
            cache_pages: Vec::new(),
            #[cfg(target_os = "android")]
            push_sent: None,
            #[cfg(target_os = "android")]
            _network_registration: network_registration,
        }
    }

    fn queue_cached_page(
        &mut self,
        channel_id: String,
        before: Option<(i64, String)>,
        older: bool,
    ) -> bool {
        let restore_epoch = self.runtime.store.cache_restore_epoch();
        let Some(receiver) = self.runtime.cache.load_channel_page_async(
            &self.runtime.server_key,
            &channel_id,
            before,
        ) else {
            self.runtime.store.cached_page_failed(&channel_id, older);
            return false;
        };
        self.cache_pages.push(PendingCachePage {
            channel_id,
            older,
            restore_epoch,
            receiver,
        });
        true
    }

    fn ensure_channel_cache_hydrated(&mut self) {
        let Some(channel_id) = self.runtime.store.channel_needing_cache() else {
            return;
        };
        self.runtime.store.mark_cache_loading(&channel_id);
        self.queue_cached_page(channel_id, None, false);
    }

    fn ensure_channel_reconciled(&mut self) {
        let Some(channel_id) = self.runtime.store.channel_needing_messages() else {
            return;
        };
        let ticket = self.runtime.store.mark_loading(&channel_id);
        self.runtime.net.send(Command::LoadMessages { ticket });
    }

    fn sync_notification_context(&self, enabled: bool, visible_server: bool, push: bool) {
        self.runtime.sync_notification_context(RuntimeNotificationView {
            server_label: self.label.clone(),
            visible_server,
            notifications_enabled: enabled,
            push_registered: push,
        });
    }

    /// Como o trilho vê este servidor.
    fn entry(&self, settings: &Settings) -> crate::ui::rail::Entry {
        crate::ui::rail::Entry {
            label: self.label.clone(),
            address: self.runtime.url.clone(),
            mentions: if settings.badge {
                self.runtime.store.mention_total()
            } else {
                0
            },
            unread: settings.badge && self.runtime.store.has_unread(),
            signed_in: self.runtime.store.screen == Screen::Chat,
            online: matches!(
                self.runtime.store.connection,
                crate::api::ws::Connection::Online
            ),
            icon: None,
        }
    }
}

fn activity_for_server(activity: &Activity) -> Activity {
    use base64::Engine as _;

    const MAX_RAW_IMAGE: usize = 190 * 1024;

    let mut wire = activity.clone();
    wire.image = activity.image.as_deref().and_then(|path| {
        if path.starts_with("data:image/") {
            return Some(path.to_owned());
        }
        let bytes = std::fs::read(path).ok()?;
        if bytes.len() > MAX_RAW_IMAGE {
            log::warn!("rich presence: activity art is too large to publish");
            return None;
        }
        let mime = match std::path::Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("jpg" | "jpeg") => "image/jpeg",
            Some("webp") => "image/webp",
            Some("gif") => "image/gif",
            _ => "image/png",
        };
        Some(format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    });
    wire
}

pub struct PapoApp {
    workspaces: Vec<Workspace>,
    /// Um banco de cache por processo, compartilhado pelos servidores.
    cache: std::sync::Arc<ClientDb>,
    /// Política e deduplicação durável de notificações do processo.
    notification: std::sync::Arc<NotificationCoordinator>,
    /// Índice do servidor na tela.
    active: usize,
    ui: UiState,
    settings: Settings,
    rich_presence: crate::rich_presence::Manager,
    system: SystemTheme,
    tokens: Tokens,
    roles: crate::ui::roles::RolesState,
    sheet: crate::ui::settings::SettingsState,
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    tray: Option<Tray>,
    #[cfg(target_os = "linux")]
    launcher: Option<Launcher>,
    #[cfg(any(target_os = "windows", target_os = "android"))]
    updater: crate::platform::update::Updater,
    #[cfg(any(target_os = "windows", target_os = "android"))]
    update_available: Option<crate::platform::update::Available>,
    #[cfg(any(target_os = "windows", target_os = "android"))]
    update_prompt_visible: bool,
    #[cfg(any(target_os = "windows", target_os = "android"))]
    update_status: Option<String>,
    #[cfg(any(target_os = "windows", target_os = "android"))]
    update_progress: Option<f32>,
    #[cfg(any(target_os = "windows", target_os = "android"))]
    update_ready: Option<std::path::PathBuf>,
    #[cfg(target_os = "android")]
    update_waiting_permission: bool,
    /// Diálogos do sistema em aberto (anexar, salvar como, escolher pasta).
    dialogs: Dialogs,
    /// Editor de recorte aberto (foto, banner, ícone do servidor).
    crop: Option<crate::ui::crop::CropEditor>,
    /// O voltar do sistema deste quadro, quando é do editor de recorte.
    crop_back: bool,
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
    /// Enquanto o servidor criado pelo botão + ainda está no modal, guarda
    /// qual servidor estava na tela para poder cancelar sem deixar lixo no trilho.
    add_server_previous: Option<usize>,
    /// Excludes WorkManager runtimes for the whole lifetime of PapoApp.
    #[cfg(target_os = "android")]
    _runtime_lease: crate::platform::runtime_lease::ForegroundLease,
}

impl PapoApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        settings.normalise();

        #[cfg(target_os = "android")]
        let runtime_lease = crate::platform::runtime_lease::ForegroundLease::acquire();

        #[cfg(target_os = "android")]
        {
            let background_servers: Vec<(String, String, bool, bool)> = settings
                .servers
                .iter()
                .map(|entry| {
                    let key = papo_core::server_key(&entry.url);
                    let enabled = settings
                        .pending_user_settings
                        .get(&key)
                        .or_else(|| settings.cached_user_settings.get(&key))
                        .map(|config| config.notifications.enabled)
                        .unwrap_or(settings.notifications);
                    let push = settings.push_devices.contains_key(&key);
                    (key, entry.url.clone(), enabled, push)
                })
                .collect();
            crate::platform::android_work::sync_periodic(
                background_servers.iter().map(|(key, url, enabled, push)| {
                    (key.as_str(), url.as_str(), *enabled, *push)
                }),
            );
        }

        #[cfg(target_os = "android")]
        if settings.notifications {
            crate::platform::android_message::ensure_permission();
        }

        let system = SystemTheme::read();
        #[cfg(not(target_os = "android"))]
        crate::render::log_adapter(cc);
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

        #[cfg(target_os = "android")]
        let glass = cc.gl.as_ref().and_then(|gl| GlassRenderer::new(gl));
        #[cfg(not(target_os = "android"))]
        let glass = cc
            .wgpu_render_state
            .as_ref()
            .and_then(GlassRenderer::new);
        if glass.is_none() {
            log::warn!("renderer sem suporte ao compositor de vidro; efeito desativado");
        }

        // Um banco de cache por processo. Abrir aqui deixa o restore
        // acontecer antes de qualquer worker de rede subir.
        let cache = crate::platform::client_db::get();
        let preview_ctx = cc.egui_ctx.clone();
        let previews = std::sync::Arc::new(papo_core::preview::PreviewCoordinator::new(
            std::sync::Arc::clone(&cache),
            std::sync::Arc::new(move || preview_ctx.request_repaint()),
        ));

        #[cfg(target_os = "linux")]
        let notifier = Notifier::spawn();
        #[cfg(target_os = "windows")]
        let notifier = Notifier::spawn(cc.egui_ctx.clone());

        #[cfg(any(target_os = "linux", target_os = "windows"))]
        let notification_sink: Option<NotificationSink> = notifier.as_ref().map(|notifier| {
            let notifier = notifier.clone();
            std::sync::Arc::new(move |envelope: papo_core::notification::NotificationEnvelope| {
                notifier.show(Notification {
                    summary: envelope.title,
                    body: envelope.body,
                    tag: Some(format!("{}\n{}", envelope.server_key, envelope.channel_id)),
                });
            }) as NotificationSink
        });

        #[cfg(target_os = "android")]
        let notification_sink: Option<NotificationSink> = Some(std::sync::Arc::new(
            |envelope: papo_core::notification::NotificationEnvelope| {
                crate::platform::android_message::show_envelope(envelope);
            },
        ));

        #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "android")))]
        let notification_sink: Option<NotificationSink> = None;

        #[cfg(target_os = "android")]
        let notification = std::sync::Arc::new(NotificationCoordinator::with_foreground_probe(
            std::sync::Arc::clone(&cache),
            notification_sink,
            std::sync::Arc::new(crate::platform::android_call::is_foreground),
        ));
        #[cfg(not(target_os = "android"))]
        let notification = std::sync::Arc::new(NotificationCoordinator::new(
            std::sync::Arc::clone(&cache),
            notification_sink,
        ));
        #[cfg(not(target_os = "android"))]
        notification.set_foreground(true);

        // Todos os servidores sobem juntos: o que chega num deles enquanto
        // outro está na tela ainda conta para o contador e a notificação.
        let mut workspaces: Vec<Workspace> = settings
            .servers
            .iter()
            .map(|entry| {
                Workspace::open(
                    entry,
                    &settings.server_marks,
                    &cc.egui_ctx,
                    &cache,
                    &notification,
                )
            })
            .collect();
        let active = settings.active.min(workspaces.len() - 1);
        for (index, workspace) in workspaces.iter().enumerate() {
            workspace.sync_notification_context(
                settings.notifications,
                index == active,
                settings
                    .push_devices
                    .contains_key(&workspace.runtime.server_key),
            );
        }

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
                        .push(Workspace::open(
                            &entry,
                            &settings.server_marks,
                            &cc.egui_ctx,
                            &cache,
                            &notification,
                        ));
                }
                workspaces[index].label = label.to_owned();
            }
            let active_key = workspaces[active].runtime.server_key.clone();
            crate::state::demo::seed(&mut workspaces[active].runtime.store, &active_key);
            // Os outros ficam com conversa por ler, para o marcador e o
            // contador aparecerem.
            let second_key = workspaces[1].runtime.server_key.clone();
            crate::state::demo::seed(&mut workspaces[1].runtime.store, &second_key);
            let third_key = workspaces[2].runtime.server_key.clone();
            crate::state::demo::seed(&mut workspaces[2].runtime.store, &third_key);
            workspaces[1].runtime.store.read_marks.clear();
            workspaces[2].runtime.store.read_marks.clear();
        }

        // O servidor que está na tela entrega o seu guardado para a interface.
        let mut ui_state = UiState::default();
        ui_state.previews = Some(std::sync::Arc::clone(&previews));
        ui_state.show_members = settings.show_members;
        ui_state.translucent = settings.translucency;
        ui_state.reveal_topic = settings.topic_reveal;
        ui_state.open_at_newest = settings.open_at_newest;
        ui_state.show_record = settings.record_button;
        ui_state.channel_emoji_monochrome = settings.channel_emoji_monochrome;
        ui_state.self_card = settings.self_card;
        ui_state.webembed_behavior = settings.webembed_offscreen;
        ui_state.webembed_scope = settings.webembed_scope;
        ui_state.webembed_float_width = settings.webembed_float_width;
        ui_state.webembed_float_pos = settings.webembed_float_pos;
        ui_state.trusted_link_hosts = settings.trusted_link_hosts.clone();
        ui_state.gif_favourites = settings.gif_favourites.clone();
        ui_state.gif_locale = match settings.lang {
            Lang::PtBr => "pt",
            Lang::En => "en",
        }
        .to_owned();
        ui_state.glass = glass;
        workspaces[active].stash.swap(&mut ui_state);

        let rich_presence =
            crate::rich_presence::Manager::new(settings.rich_presence.clone(), cc.egui_ctx.clone());

        #[cfg(target_os = "linux")]
        crate::platform::shutdown::spawn(cc.egui_ctx.clone());

        Self {
            workspaces,
            cache,
            notification,
            active,
            ui: ui_state,
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            tray: Tray::spawn(cc.egui_ctx.clone(), tray_labels(&settings)),
            #[cfg(target_os = "linux")]
            launcher: Launcher::spawn(),
            #[cfg(any(target_os = "windows", target_os = "android"))]
            updater: crate::platform::update::Updater::new(),
            #[cfg(any(target_os = "windows", target_os = "android"))]
            update_available: None,
            #[cfg(any(target_os = "windows", target_os = "android"))]
            update_prompt_visible: false,
            #[cfg(any(target_os = "windows", target_os = "android"))]
            update_status: None,
            #[cfg(any(target_os = "windows", target_os = "android"))]
            update_progress: None,
            #[cfg(any(target_os = "windows", target_os = "android"))]
            update_ready: None,
            #[cfg(target_os = "android")]
            update_waiting_permission: false,
            dialogs: Dialogs::default(),
            crop: None,
            crop_back: false,
            focused: true,
            quitting: false,
            settings,
            rich_presence,
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
                                "atividade" | "activity" => crate::ui::settings::AppPane::Activity,
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
            add_server_previous: None,
            #[cfg(target_os = "android")]
            _runtime_lease: runtime_lease,
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
        #[cfg(target_os = "windows")]
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        #[cfg(not(target_os = "windows"))]
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
    }

    /// Fechar a janela só encerra quando a bandeja não está no caminho.
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn handle_window_lifecycle(&mut self, ctx: &egui::Context) {
        self.focused = ctx.input(|input| input.viewport().focused).unwrap_or(true);

        if self
            .tray
            .as_ref()
            .is_some_and(Tray::take_quit_requested)
        {
            self.quitting = true;
        }

        #[cfg(target_os = "linux")]
        if crate::platform::shutdown::requested() {
            // A session is going away. Never turn the compositor's shutdown
            // close into "minimize to tray", otherwise Papo can hold logout up.
            self.quit(ctx);
        }

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
                TrayCommand::RestartRichPresence => self.rich_presence.restart(),
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
                .map(|ws| ws.runtime.store.mention_total())
                .sum()
        } else {
            0
        };
        let unread = self.settings.badge
            && self.workspaces.iter().any(|ws| ws.runtime.store.has_unread());

        if let Some(tray) = &self.tray {
            tray.set_badge(mentions, unread);
            tray.set_labels(tray_labels(&self.settings));
        }
        #[cfg(target_os = "linux")]
        if let Some(launcher) = &self.launcher {
            launcher.set(Badge {
                count: mentions,
                urgent: unread && mentions == 0,
            });
        }
    }

    #[cfg(any(target_os = "windows", target_os = "android"))]
    fn pump_updater(&mut self, ctx: &egui::Context) {
        use crate::platform::update::Event;

        #[cfg(target_os = "windows")]
        let _ = ctx;

        #[cfg(target_os = "android")]
        if self.update_waiting_permission && crate::platform::update::can_install_packages() {
            self.update_waiting_permission = false;
            if let Some(release) = self.update_available.clone() {
                self.update_status = Some(self.settings.lang.strings().update_downloading.to_owned());
                self.update_progress = Some(0.0);
                self.updater.download(release);
            }
        }

        while let Some(event) = self.updater.poll() {
            match event {
                Event::Current => {
                    self.update_status = Some(self.settings.lang.strings().update_current.to_owned());
                    self.update_available = None;
                    self.update_prompt_visible = false;
                    self.settings.dismissed_update_version = None;
                    #[cfg(target_os = "android")]
                    {
                        self.update_progress = None;
                    }
                    self.update_ready = None;
                }
                Event::Available(release) => {
                    self.update_status = Some(format!(
                        "{} {}",
                        self.settings.lang.strings().update_available,
                        release.version
                    ));
                    self.update_prompt_visible = self
                        .settings
                        .dismissed_update_version
                        .as_deref()
                        != Some(release.version.as_str());
                    self.update_available = Some(release);
                }
                #[cfg(target_os = "android")]
                Event::Progress { downloaded, total } => {
                    self.update_progress = total
                        .filter(|total| *total > 0)
                        .map(|total| (downloaded as f32 / total as f32).clamp(0.0, 1.0));
                    ctx.request_repaint();
                }
                Event::Ready { release, installer } => {
                    self.update_available = Some(release);
                    #[cfg(target_os = "android")]
                    {
                        self.update_progress = Some(1.0);
                    }
                    self.update_ready = Some(installer);
                    self.update_status = None;
                }
                Event::Error(error) => {
                    log::warn!("atualização: {error}");
                    #[cfg(target_os = "android")]
                    {
                        self.update_progress = None;
                    }
                    self.update_ready = None;
                    self.update_status = Some(format!(
                        "{} {error}",
                        self.settings.lang.strings().update_failed
                    ));
                }
            }
        }
    }

    #[cfg(any(target_os = "windows", target_os = "android"))]
    fn update_prompt(&mut self, ctx: &egui::Context) {
        if !self.update_prompt_visible {
            return;
        }
        let Some(release) = self.update_available.clone() else {
            self.update_prompt_visible = false;
            return;
        };
        let s = self.settings.lang.strings();
        let compact = ctx.content_rect().width() < shell::COMPACT_BREAKPOINT;
        let width = if compact {
            (ctx.content_rect().width() - 32.0).clamp(280.0, 520.0)
        } else {
            520.0
        };
        let mut dismiss = false;
        egui::Window::new(format!("{} {}", s.update_available, release.version))
            .id(egui::Id::new("papo-update-prompt"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .collapsible(false)
            .resizable(!compact)
            .fixed_size(if compact {
                egui::vec2(width, (ctx.content_rect().height() * 0.72).clamp(360.0, 620.0))
            } else {
                egui::vec2(width, 420.0)
            })
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new(s.update_release_notes).strong());
                });
                ui.add_space(6.0);
                let notes_height = (ui.available_height() - 58.0).max(120.0);
                egui::ScrollArea::vertical()
                    .id_salt("update-release-notes")
                    .max_height(notes_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.label(&release.notes);
                    });
                ui.add_space(10.0);
                ui.horizontal_wrapped(|ui| {
                    let buttons_width = 318.0_f32.min(ui.available_width());
                    ui.add_space(((ui.available_width() - buttons_width) * 0.5).max(0.0));
                    if ui.button(s.update_now).clicked() {
                        self.settings.dismissed_update_version = Some(release.version.clone());
                        #[cfg(target_os = "android")]
                        if !crate::platform::update::can_install_packages() {
                            self.update_waiting_permission = true;
                            crate::platform::update::request_install_permission();
                            self.update_status = Some(s.update_waiting_permission.to_owned());
                            dismiss = true;
                        } else {
                            self.update_status = Some(s.update_downloading.to_owned());
                            self.update_progress = Some(0.0);
                            self.updater.download(release.clone());
                            dismiss = true;
                        }

                        #[cfg(target_os = "windows")]
                        {
                            self.update_status = Some(s.update_downloading.to_owned());
                            self.updater.download(release.clone());
                            dismiss = true;
                        }
                    }
                    if ui.button(s.update_later).clicked() {
                        self.settings.dismissed_update_version = Some(release.version.clone());
                        dismiss = true;
                    }
                    if ui.button(s.update_open_release).clicked() {
                        crate::platform::links::open_url(&release.release_url);
                    }
                });
            });
        if dismiss {
            self.update_prompt_visible = false;
        }
    }

    #[cfg(any(target_os = "windows", target_os = "android"))]
    fn project_update_pill(&mut self) {
        let downloading = self.updater.downloading();
        let ready = self.update_ready.is_some();
        #[cfg(target_os = "android")]
        let waiting_permission = self.update_waiting_permission;
        #[cfg(target_os = "windows")]
        let waiting_permission = false;

        self.ui.update_pill = if downloading {
            Some(shell::UpdatePill::Downloading(self.update_progress))
        } else if ready {
            Some(shell::UpdatePill::Ready)
        } else if waiting_permission {
            Some(shell::UpdatePill::WaitingPermission)
        } else if self.update_available.is_some() && !self.update_prompt_visible {
            Some(shell::UpdatePill::Available)
        } else {
            None
        };
        self.ui.update_pill_clicked = false;
    }

    #[cfg(any(target_os = "windows", target_os = "android"))]
    fn handle_update_pill_click(&mut self, ctx: &egui::Context) {
        if let Some(installer) = self.update_ready.clone() {
            match crate::platform::update::launch(&installer) {
                Ok(()) => {
                    self.update_status = None;
                    #[cfg(target_os = "windows")]
                    self.quit(ctx);
                }
                Err(error) => {
                    self.update_status = Some(format!(
                        "{} {error}",
                        self.settings.lang.strings().update_failed
                    ));
                }
            }
            return;
        }

        if self.updater.downloading() {
            return;
        }
        #[cfg(target_os = "android")]
        if self.update_waiting_permission {
            return;
        }

        if self.update_available.is_some() {
            self.settings.dismissed_update_version = None;
            self.update_prompt_visible = true;
            ctx.request_repaint();
        }
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
            self.workspaces[index].runtime.store.error = Some(s.invalid_server_address.to_owned());
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
                .find(|(other, ws)| *other != index && normalise_server_url(&ws.runtime.url) == url)
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

        if url != self.workspaces[index].runtime.url {
            self.reopen(index, url, ctx);
        }

        let ws = &mut self.workspaces[index];
        ws.runtime.store.busy = true;
        ws.runtime.store.error = None;
        ws.runtime.net.send(if register {
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
        self.workspaces[index].runtime.store.busy = true;
        self.workspaces[index].runtime.store.error = None;
        self.workspaces[index].runtime
            .net
            .send(Command::LoginServer { password });
    }

    /// Reabre um servidor num endereço novo, jogando fora a conexão antiga.
    fn reopen(&mut self, index: usize, url: String, ctx: &egui::Context) {
        let form = self.workspaces[index].form.clone();
        let old_key = self.workspaces[index].runtime.server_key.clone();
        let entry = ServerEntry::new(url);
        let mut fresh = Workspace::open(
            &entry,
            &self.settings.server_marks,
            ctx,
            &self.cache,
            &self.notification,
        );
        fresh.form = AuthForm {
            server_url: entry.url.clone(),
            ..form
        };
        // O endereço velho some de propósito: o cache dele não pode aparecer
        // sob a chave nova só porque o servidor é o mesmo.
        self.notification.remove_context(&old_key);
        self.cache.clear_server(&old_key);
        crate::media::clear_server_media_cache(&old_key);
        self.settings.servers[index] = entry;
        // O servidor na tela devolve o guardado para o substituto, ou a
        // interface ficaria com a mídia de uma conexão que já morreu.
        if index == self.active {
            self.workspaces[index].stash.swap(&mut self.ui);
            fresh.stash.swap(&mut self.ui);
        }
        self.workspaces[index] = fresh;
        self.sync_notification_contexts();
    }

    /// Passa a mostrar outro servidor. Os dois seguem conectados; o que troca
    /// é qual deles ocupa a janela.
    fn activate(&mut self, index: usize, ctx: &egui::Context) {
        if index == self.active || index >= self.workspaces.len() {
            return;
        }
        self.persist_active_drafts(true);
        if self.settings.webembed_scope == crate::webembed::FloatScope::CurrentChannel {
            self.ui.webembed.destroy_active();
        }
        // Nunca materializa um rascunho guardado de outra conta, nem por um
        // quadro enquanto o load da conta atual ainda não aconteceu.
        let target_owner = self.workspaces[index]
            .runtime
            .cached_owner()
            .unwrap_or_default()
            .to_owned();
        if target_owner.is_empty()
            || self.workspaces[index].stash.drafts.owner() != Some(target_owner.as_str())
        {
            let stash = &mut self.workspaces[index].stash;
            stash.composer.clear();
            stash.composer_mentions.clear();
            stash.replying = None;
            stash.reply_notify = self.settings.reply_notifications;
            stash.drafts = Default::default();
        }
        // O que estava na tela recolhe o seu; o novo entrega o dele.
        let previous = self.active;
        self.workspaces[previous].stash.swap(&mut self.ui);
        self.workspaces[index].stash.swap(&mut self.ui);
        self.active = index;
        self.settings.active = index;
        self.settings.server_url = self.workspaces[index].runtime.url.clone();
        self.sync_notification_contexts();
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
        let mut workspace = Workspace::open(
            &entry,
            &self.settings.server_marks,
            ctx,
            &self.cache,
            &self.notification,
        );
        // Não herda o endereço padrão nem a sessão dele. O cartão nasce
        // realmente vazio e só cria conexão com o servidor digitado no envio.
        workspace.form.server_url.clear();
        workspace.runtime.store.screen = Screen::Auth;
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
        let key = crate::state::server_key(&self.workspaces[index].runtime.url);
        self.settings.server_marks.remove(&key);
        if let Some(token) = self.settings.push_devices.remove(&key) {
            self.workspaces[index].runtime.release_push_device(token);
        }
        self.workspaces[index].runtime.forget_server();
        self.workspaces.remove(index);
        self.settings.servers.remove(index);

        self.active = previous.min(self.workspaces.len() - 1);
        self.settings.active = self.active;
        self.settings.server_url = self.workspaces[self.active].runtime.url.clone();
        self.workspaces[self.active].stash.swap(&mut self.ui);
        self.sync_notification_contexts();
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
        let key = crate::state::server_key(&self.workspaces[index].runtime.url);
        self.settings.server_marks.remove(&key);
        if let Some(token) = self.settings.push_devices.remove(&key) {
            self.workspaces[index].runtime.release_push_device(token);
        }
        self.workspaces[index].runtime.forget_server();
        crate::media::clear_server_media_cache(&key);
        self.workspaces.remove(index);
        self.settings.servers.remove(index);

        let active = if self.active > index {
            self.active - 1
        } else {
            self.active.min(self.workspaces.len() - 1)
        };
        self.active = active;
        self.settings.active = active;
        self.settings.server_url = self.workspaces[active].runtime.url.clone();
        if was_active {
            self.workspaces[active].stash.swap(&mut self.ui);
        }
        self.sync_notification_contexts();
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

        ws.ensure_channel_cache_hydrated();
        ws.ensure_channel_reconciled();

        // Foco sozinho não significa leitura. Só as linhas realmente expostas
        // pela viewport no frame anterior avançam a fronteira durável.
        if focused && !ws.runtime.store.selected_channel.is_empty() {
            let channel_id = ws.runtime.store.selected_channel.clone();
            if std::mem::take(&mut self.ui.read_span_break) {
                ws.runtime.store.reset_read_span(&channel_id);
            }
            let visible = self.ui.visible_message_ids.clone();
            if !visible.is_empty() {
                ws.runtime
                    .store
                    .observe_visible_messages(&channel_id, &visible);

                // O backend recebe exatamente as notificações cujas mensagens
                // ficaram visíveis; saltar por cima não confirma o intervalo.
                let ids = ws
                    .runtime
                    .store
                    .take_seen_notifications(&channel_id, &visible);
                if !ids.is_empty() && !ws.runtime.store.me.is_empty() {
                    #[cfg(target_os = "android")]
                    crate::platform::android_message::clear_channel(&ws.runtime.url, &channel_id);
                    ws.runtime.net.send(Command::MarkNotificationsRead {
                        user_id: ws.runtime.store.me.clone(),
                        ids,
                    });
                }
            }
        }

        // Um evento de digitação a cada três segundos basta para o servidor.
        if typed {
            let now = std::time::Instant::now();
            let stale = ws
                .typing_sent
                .is_none_or(|last| now.duration_since(last).as_secs() >= 3);
            if stale && !ws.runtime.store.selected_channel.is_empty() {
                ws.typing_sent = Some(now);
                ws.runtime.net.send(Command::Typing {
                    channel_id: ws.runtime.store.selected_channel.clone(),
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
                if self.workspaces[self.active].runtime.store.can_manage_channels() {
                    self.sheet.open_new_channel();
                }
                return;
            }
            ChatAction::EditChannel(id) => {
                if !self.workspaces[self.active].runtime.store.can_manage_channels() {
                    return;
                }
                if let Some(channel) = self.workspaces[self.active].runtime.store.channel(id).cloned() {
                    self.sheet.open_edit_channel(&channel);
                    let net = &self.workspaces[self.active].runtime.net;
                    net.send(Command::LoadRoles);
                    net.send(Command::LoadChannelPermissions {
                        channel_id: id.clone(),
                    });
                }
                return;
            }
            ChatAction::RequestDeleteChannel(id) => {
                if !self.workspaces[self.active].runtime.store.can_manage_channels() {
                    return;
                }
                if let Some(channel) = self.workspaces[self.active].runtime.store.channel(id).cloned() {
                    self.sheet.open_delete_channel(&channel);
                }
                return;
            }
            ChatAction::EditProfile => {
                self.sheet.open_account();
                return;
            }
            ChatAction::MarkServerRead => {
                let ws = &mut self.workspaces[self.active];
                ws.runtime.store.mark_all_read();
                let ids = ws.runtime.store.take_all_open_notifications();
                if !ids.is_empty() && !ws.runtime.store.me.is_empty() {
                    ws.runtime.net.send(Command::MarkNotificationsRead {
                        user_id: ws.runtime.store.me.clone(),
                        ids,
                    });
                }
                return;
            }
            ChatAction::LeaveServer => {
                let active = self.active;
                self.remove_server(active, ctx);
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
                let title = self.workspaces[self.active].runtime
                    .store
                    .channel(&channel_id)
                    .map(|channel| channel.name.clone())
                    .unwrap_or_else(|| "Papo".to_owned());
                crate::platform::android_call::start_service(&title);
            }
            let ws = &mut self.workspaces[self.active];
            ws.runtime.store.selected_channel = channel_id.clone();
            let attempt = ws.runtime.store.call.joining(channel_id.clone());
            ws.runtime.net.send(Command::JoinVoice {
                channel_id,
                attempt,
            });
            return;
        }
        let s = self.settings.lang.strings();
        let ws = &mut self.workspaces[self.active];
        match action {
            ChatAction::LoadOlderMessages => {
                let channel_id = ws.runtime.store.selected_channel.clone();
                if let Some(before) = ws.runtime.store.begin_load_cached_older(&channel_id) {
                    ws.queue_cached_page(channel_id, Some(before), true);
                } else if let Some((since, last_id)) =
                    ws.runtime.store.begin_load_older(&channel_id)
                {
                    ws.runtime.net.send(Command::LoadOlderMessages {
                        channel_id,
                        since,
                        last_id,
                    });
                }
            }
            ChatAction::Send {
                content,
                reply_to,
                notify_reply,
                attachments,
            } => {
                let channel_id = ws.runtime.store.selected_channel.clone();
                let wire_content = content;
                // Sem canal não há para onde mandar. Engolir a mensagem aqui
                // fazia o envio parecer quebrado: a caixa esvaziava e nada
                // acontecia, sem uma palavra de explicação.
                if channel_id.is_empty() {
                    ws.runtime.store.error = Some(s.no_channel_selected.to_owned());
                    if attachments.is_empty() {
                        let (visible, bindings) =
                            ws.runtime.store.display_mentions_with_bindings(&wire_content);
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
                    let owner_user_id = ws.runtime.store.me.clone();
                    if owner_user_id.is_empty() {
                        ws.runtime.store.error =
                            Some("sessão ainda não verificada para enviar".to_owned());
                        let (visible, bindings) =
                            ws.runtime.store.display_mentions_with_bindings(&wire_content);
                        self.ui.composer = visible;
                        self.ui.composer_mentions = bindings;
                        self.ui.replying = reply_to;
                        self.ui.reply_notify = notify_reply;
                        return;
                    }
                    self.ui.capture_draft(&channel_id);
                    let draft_ops = self.ui.drafts.take_persistence_ops(&owner_user_id, true);
                    if !draft_ops.is_empty() {
                        self.cache.submit(&ws.runtime.server_key, draft_ops);
                    }
                    ws.runtime.net.send(Command::QueueMessage {
                        local_id: papo_core::cache::new_local_id(),
                        owner_user_id,
                        channel_id,
                        content: wire_content,
                        reply_to,
                        notify_reply,
                        created_at: papo_core::cache::now_millis(),
                    });
                } else {
                    let owner_user_id = ws.runtime.store.me.clone();
                    if !owner_user_id.is_empty() {
                        self.ui.capture_draft(&channel_id);
                        let draft_ops =
                            self.ui.drafts.take_persistence_ops(&owner_user_id, true);
                        if !draft_ops.is_empty() {
                            self.cache.submit(&ws.runtime.server_key, draft_ops);
                        }
                    }
                    if attachments.iter().any(|upload| upload.anonymize) {
                        // Reescrever imagem custa; fora da thread da interface.
                        let sender = ws.runtime.net.sender();
                        std::thread::spawn(move || {
                            let dir = crate::media::cache_root().join("anonymized");
                            let attachments = attachments
                                .into_iter()
                                .filter_map(|upload| {
                                    if !upload.anonymize {
                                        return Some(upload);
                                    }
                                    match crate::platform::anonymize::anonymize(&upload, &dir) {
                                        Ok(done) => Some(done.upload),
                                        // Nunca cair para o original: o usuário
                                        // pediu anonimato.
                                        Err(error) => {
                                            log::warn!("anonimizar {}: {error}", upload.name);
                                            None
                                        }
                                    }
                                })
                                .collect();
                            sender.send(Command::SendMessage {
                                channel_id,
                                content: wire_content,
                                reply_to,
                                notify_reply,
                                attachments,
                            });
                        });
                    } else {
                        ws.runtime.net.send(Command::SendMessage {
                            channel_id,
                            content: wire_content,
                            reply_to,
                            notify_reply,
                            attachments,
                        });
                    }
                }
            }
            ChatAction::Edit {
                message_id,
                content,
            } => ws.runtime.net.send(Command::EditMessage {
                message_id,
                content,
            }),
            ChatAction::Delete(message_id) => {
                ws.runtime.net.send(Command::DeleteMessage { message_id })
            }
            ChatAction::RetryOutgoing(local_id) => {
                ws.runtime.net.send(Command::RetryOutgoing {
                    local_id,
                    owner_user_id: ws.runtime.store.me.clone(),
                });
            }
            ChatAction::DismissOutgoing(local_id) => {
                ws.runtime.net.send(Command::DismissOutgoing {
                    local_id,
                    owner_user_id: ws.runtime.store.me.clone(),
                });
            }
            ChatAction::React {
                message_id,
                emoji,
                add,
            } => {
                let channel_id = ws.runtime
                    .store
                    .message(&message_id)
                    .map(|message| message.channel_id.clone())
                    .unwrap_or_else(|| ws.runtime.store.selected_channel.clone());
                // A reação aparece na hora; o contador certo vem pelo evento.
                ws.runtime.store.set_reaction_local(&message_id, &emoji, add);
                ws.runtime.net.send(Command::React {
                    channel_id,
                    message_id,
                    emoji: emoji.request(),
                    add,
                });
            }
            ChatAction::Pin { message_id, pin } => {
                let channel_id = ws.runtime
                    .store
                    .message(&message_id)
                    .map(|message| message.channel_id.clone())
                    .unwrap_or_else(|| ws.runtime.store.selected_channel.clone());
                ws.runtime.net.send(Command::Pin {
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
            ChatAction::SaveCachedImage { path, name } => match self.settings.downloads.clone() {
                DownloadMode::Ask => self.dialogs.save_cached_as(
                    ctx.clone(),
                    path,
                    name,
                    files::downloads_dir(),
                ),
                DownloadMode::Folder(dir) => {
                    let dir = if dir.is_dir() { dir } else { files::downloads_dir() };
                    let dest = files::unique_path(&dir, &name);
                    if let Some(parent) = dest.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(error) = std::fs::copy(&path, &dest) {
                        log::warn!("não deu para salvar imagem de link: {error}");
                    }
                }
            },
            ChatAction::NewChannel
            | ChatAction::EditChannel(_)
            | ChatAction::RequestDeleteChannel(_) => unreachable!("tratadas antes do match"),
            ChatAction::CreateChannel { name, kind, topic } => {
                ws.runtime.net.send(Command::CreateChannel { name, kind, topic })
            }
            ChatAction::UpdateChannel {
                channel_id,
                name,
                topic,
            } => ws.runtime.net.send(Command::UpdateChannel {
                channel_id,
                name,
                topic,
            }),
            ChatAction::SetChannelPermissions {
                channel_id,
                role_id,
                permissions,
            } => ws.runtime.net.send(Command::SetChannelPermissions {
                channel_id,
                role_id,
                permissions,
            }),
            ChatAction::RemoveChannelRolePermission { channel_id, role_id } => {
                ws.runtime
                    .net
                    .send(Command::RemoveChannelRolePermission { channel_id, role_id })
            }
            ChatAction::DeleteChannel(channel_id) => {
                ws.runtime.net.send(Command::DeleteChannel { channel_id })
            }
            ChatAction::ChannelNotifications {
                channel_id,
                setting,
            } => {
                if let Some(channel) = ws
                    .runtime
                    .store
                    .channels
                    .iter_mut()
                    .find(|channel| channel.id == channel_id)
                {
                    channel.notification_settings = setting.to_owned();
                }
                ws.runtime.net.send(Command::SetChannelNotifications {
                    channel_id,
                    setting: setting.to_owned(),
                });
            }
            ChatAction::MoveChannel {
                channel_id,
                old_position,
                new_position,
                parent_id,
            } => ws.runtime.net.send(Command::MoveChannel {
                channel_id,
                old_position,
                new_position,
                parent_id,
            }),
            ChatAction::BanUser { user_id, banned } => {
                ws.runtime.net.send(Command::BanUser { user_id, banned })
            }
            ChatAction::ResetUser(user_id) => ws.runtime.net.send(Command::ResetUser { user_id }),
            ChatAction::LoadProfile(user_id) => ws.runtime.net.send(Command::LoadProfile { user_id }),
            ChatAction::OpenDirectMessage(user_id) => {
                if !self.ui.dm_surface
                    && ws.runtime.store.channel(&ws.runtime.store.selected_channel).is_some_and(|channel| channel.kind != crate::state::ChannelKind::Direct)
                {
                    self.ui.last_server_channel = ws.runtime.store.selected_channel.clone();
                }
                self.ui.dm_surface = true;
                if let Some(dm) = ws
                    .runtime
                    .store
                    .direct_messages
                    .iter()
                    .find(|dm| dm.user.id == user_id)
                {
                    ws.runtime.store.selected_channel = dm.id.clone();
                }
                ws.runtime.net.send(Command::OpenDirectMessage { user_id });
            }
            ChatAction::HideDirectMessage(dm_id) => {
                ws.runtime.net.send(Command::HideDirectMessage { dm_id: dm_id.clone() });
                if ws.runtime.store.selected_channel == dm_id {
                    ws.runtime.store.selected_channel = ws
                        .runtime
                        .store
                        .direct_messages
                        .iter()
                        .find(|dm| dm.id != dm_id)
                        .map(|dm| dm.id.clone())
                        .unwrap_or_default();
                }
            }
            ChatAction::SetUserBlocked { user_id, blocked } => {
                ws.runtime
                    .net
                    .send(Command::SetUserBlocked { user_id, blocked });
            }
            ChatAction::SetPresence(status) => ws.runtime.net.send(Command::SetStatus { status }),
            ChatAction::EditProfile | ChatAction::MarkServerRead | ChatAction::LeaveServer => {}
            // Entrar já foi tratado antes do `match`, porque mexe em todos
            // os servidores de uma vez.
            ChatAction::JoinVoice(_) => {}
            ChatAction::LeaveVoice => leave_call(ws),
            ChatAction::ToggleMute => {
                let muted = !ws.runtime.store.call.muted;
                ws.runtime.store.call.muted = muted;
                if let Some(call) = &ws.call {
                    call.set_muted(muted);
                }
            }
            ChatAction::ToggleCamera => {
                let on = !ws.runtime.store.call.camera;
                ws.runtime.store.call.camera = on;
                if let Some(call) = &ws.call {
                    call.set_camera(on);
                }
            }
            ChatAction::CollapseCall(collapsed) => ws.runtime.store.call.collapsed = collapsed,
            ChatAction::FloatCall(floating) => {
                ws.runtime.store.call.floating = floating;
                ws.runtime.store.call.collapsed = floating;
                if floating {
                    ws.runtime.store.call.popped_out = false;
                }
            }
            // Voltar para a call é ir ao canal dela, como o clique que
            // levou na primeira vez — e abrir a folha se estava encolhida.
            ChatAction::OpenCall => {
                if !ws.runtime.store.call.channel_id.is_empty() {
                    ws.runtime.store.selected_channel = ws.runtime.store.call.channel_id.clone();
                    ws.runtime.store.call.collapsed = false;
                    ws.runtime.store.call.floating = false;
                }
            }
            ChatAction::PopOutCall(out) => {
                ws.runtime.store.call.popped_out = out;
                if out {
                    ws.runtime.store.call.floating = false;
                }
            }
            ChatAction::Search {
                request,
                cursor,
                append,
            } => {
                ws.runtime.store.searching = true;
                ws.runtime.net.send(Command::Search {
                    request,
                    cursor,
                    append,
                });
            }
            ChatAction::LoadReactionDetails {
                channel_id,
                message_id,
                cursor,
                append,
            } => {
                if ws.runtime.store.begin_reaction_details(&message_id) {
                    ws.runtime.net.send(Command::LoadReactionDetails {
                        channel_id,
                        message_id,
                        cursor,
                        append,
                    });
                }
            }
            ChatAction::PickFiles => self.dialogs.pick_files(ctx.clone()),
            ChatAction::PickGallery => self.dialogs.pick_gallery(ctx.clone()),
            ChatAction::CaptureMedia => self.dialogs.capture_media(ctx.clone()),
            ChatAction::OpenExternally(path) => files::open_path(&path),
        }
    }

    /// A demonstração responde sozinha: sem rede, o estado é a verdade.
    fn handle_chat_demo(&mut self, ctx: &egui::Context, action: ChatAction) {
        let ws = &mut self.workspaces[self.active];
        match action {
            ChatAction::Send { content, reply_to, .. } => {
                let channel_id = ws.runtime.store.selected_channel.clone();
                let message_id = ws.runtime.store.push_pending(&channel_id, &content, reply_to);
                ws.runtime.store.set_message_pending_local(&message_id, false);
            }
            ChatAction::Edit {
                message_id,
                content,
            } => {
                ws.runtime.store.edit_message_local(&message_id, content);
            }
            ChatAction::Delete(message_id) => {
                ws.runtime.store.delete_message_local(&message_id);
            }
            ChatAction::RetryOutgoing(_) => {}
            ChatAction::DismissOutgoing(message_id) => {
                ws.runtime.store.delete_message_local(&message_id);
            }
            ChatAction::React {
                message_id,
                emoji,
                add,
            } => {
                ws.runtime.store.set_reaction_local(&message_id, &emoji, add);
            }
            ChatAction::Pin { message_id, pin } => {
                ws.runtime.store.set_message_pinned_local(&message_id, pin);
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
            ChatAction::SaveCachedImage { path, name } => match self.settings.downloads.clone() {
                DownloadMode::Ask => self.dialogs.save_cached_as(
                    ctx.clone(),
                    path,
                    name,
                    files::downloads_dir(),
                ),
                DownloadMode::Folder(dir) => {
                    let dir = if dir.is_dir() { dir } else { files::downloads_dir() };
                    let dest = files::unique_path(&dir, &name);
                    if let Some(parent) = dest.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(error) = std::fs::copy(&path, &dest) {
                        log::warn!("não deu para salvar imagem de link: {error}");
                    }
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
                let position = ws.runtime.store.channels.len() as i32;
                let id = format!("demo-channel-{position}");
                ws.runtime.store.channels.push(Channel {
                    id: id.clone(),
                    name,
                    kind,
                    topic,
                    position,
                    permissions: Vec::new(),
                    notification_settings: "only_mentions".to_owned(),
                    parent_id: None,
                    unread: false,
                    mentions: 0,
                });
                if kind == ChannelKind::Text {
                    ws.runtime.store.selected_channel = id;
                }
            }
            ChatAction::UpdateChannel {
                channel_id,
                name,
                topic,
            } => {
                if let Some(channel) = ws.runtime
                    .store
                    .channels
                    .iter_mut()
                    .find(|channel| channel.id == channel_id)
                {
                    channel.name = name;
                    channel.topic = topic;
                }
            }
            ChatAction::SetChannelPermissions {
                channel_id,
                role_id,
                permissions,
            } => {
                let role_name = ws
                    .runtime
                    .store
                    .roles
                    .iter()
                    .find(|role| role.id == role_id)
                    .map(|role| role.name.clone())
                    .unwrap_or_default();
                if let Some(channel) = ws
                    .runtime
                    .store
                    .channels
                    .iter_mut()
                    .find(|channel| channel.id == channel_id)
                {
                    if let Some(entry) = channel
                        .permissions
                        .iter_mut()
                        .find(|entry| entry.role_id == role_id)
                    {
                        entry.permissions = permissions;
                        entry.role_name = role_name;
                    } else {
                        channel.permissions.push(crate::api::models::ChannelPermissionEntry {
                            role_id,
                            role_name,
                            permissions,
                        });
                    }
                }
            }
            ChatAction::RemoveChannelRolePermission { channel_id, role_id } => {
                if let Some(channel) = ws
                    .runtime
                    .store
                    .channels
                    .iter_mut()
                    .find(|channel| channel.id == channel_id)
                {
                    channel.permissions.retain(|entry| entry.role_id != role_id);
                }
            }
            // A call de mentira não abre microfone nenhum: serve para o
            // desenho da grade, da pastilha e da folha.
            ChatAction::JoinVoice(channel_id) => {
                ws.runtime.store.selected_channel = channel_id.clone();
                ws.runtime.store.call.joining(channel_id.clone());
                ws.runtime.store.call.joined(
                    &channel_id,
                    crate::state::demo::call_members(),
                    vec!["u-ana".to_owned()],
                );
            }
            ChatAction::LeaveVoice => ws.runtime.store.call.left(),
            ChatAction::ToggleMute => ws.runtime.store.call.muted = !ws.runtime.store.call.muted,
            ChatAction::ToggleCamera => ws.runtime.store.call.camera = !ws.runtime.store.call.camera,
            ChatAction::CollapseCall(collapsed) => ws.runtime.store.call.collapsed = collapsed,
            ChatAction::FloatCall(floating) => {
                ws.runtime.store.call.floating = floating;
                ws.runtime.store.call.collapsed = floating;
                if floating {
                    ws.runtime.store.call.popped_out = false;
                }
            }
            // Voltar para a call é ir ao canal dela, como o clique que
            // levou na primeira vez — e abrir a folha se estava encolhida.
            ChatAction::OpenCall => {
                if !ws.runtime.store.call.channel_id.is_empty() {
                    ws.runtime.store.selected_channel = ws.runtime.store.call.channel_id.clone();
                    ws.runtime.store.call.collapsed = false;
                }
            }
            ChatAction::PopOutCall(out) => ws.runtime.store.call.popped_out = out,
            ChatAction::DeleteChannel(id) => {
                ws.runtime.store.channels.retain(|channel| channel.id != id);
                if ws.runtime.store.selected_channel == id {
                    ws.runtime.store.selected_channel = ws.runtime
                        .store
                        .channels
                        .first()
                        .map(|channel| channel.id.clone())
                        .unwrap_or_default();
                }
            }
            ChatAction::ChannelNotifications {
                channel_id,
                setting,
            } => {
                if let Some(channel) = self.workspaces[self.active]
                    .runtime
                    .store
                    .channels
                    .iter_mut()
                    .find(|channel| channel.id == channel_id)
                {
                    channel.notification_settings = setting.to_owned();
                }
            }
            // Mover na demonstração muda a ordem aqui mesmo: é como se vê o
            // arrastar e soltar sem servidor.
            ChatAction::MoveChannel {
                channel_id,
                new_position,
                parent_id,
                ..
            } => {
                self.workspaces[self.active]
                    .runtime
                    .store
                    .move_channel_local(&channel_id, new_position, parent_id);
            }
            // Sem rede na demonstração: estas ações não têm efeito local.
            ChatAction::LoadOlderMessages
            | ChatAction::OpenDirectMessage(_)
            | ChatAction::HideDirectMessage(_)
            | ChatAction::SetUserBlocked { .. }
            | ChatAction::BanUser { .. }
            | ChatAction::ResetUser(_)
            | ChatAction::LoadProfile(_)
            | ChatAction::EditProfile
            | ChatAction::MarkServerRead
            | ChatAction::LeaveServer
            | ChatAction::Search { .. }
            | ChatAction::LoadReactionDetails { .. } => {}
            // A presença muda na hora, sem servidor para confirmar.
            ChatAction::SetPresence(status) => {
                let store = &mut self.workspaces[self.active].runtime.store;
                let me = store.me.clone();
                if let Some(member) = store.members.iter_mut().find(|member| member.id == me) {
                    member.presence = match status.as_deref() {
                        Some("away") => crate::state::Presence::Away,
                        Some("busy") => crate::state::Presence::Busy,
                        _ => crate::state::Presence::Online,
                    };
                }
            }
            ChatAction::PickFiles => self.dialogs.pick_files(ctx.clone()),
            ChatAction::PickGallery => self.dialogs.pick_gallery(ctx.clone()),
            ChatAction::CaptureMedia => self.dialogs.capture_media(ctx.clone()),
            ChatAction::OpenExternally(path) => files::open_path(&path),
        }
    }

    /// Aplica o nível a **todos** os `MediaStore` — o da tela e o de cada
    /// servidor guardado. Um servidor que não está visível não pode escapar
    /// do trim só porque outro está na frente.
    #[cfg(target_os = "android")]
    fn trim_all_media(&mut self, level: crate::media::TrimLevel) {
        use crate::platform::memory_pressure::TrimTarget;
        crate::platform::memory_pressure::trim_all(
            level,
            std::iter::once(&mut self.ui.media as &mut dyn TrimTarget).chain(
                self.workspaces
                    .iter_mut()
                    .map(|ws| &mut ws.stash.media as &mut dyn TrimTarget),
            ),
        );
        self.ui.webembed.trim(level);
    }

    /// Recolhe a pressão de memória publicada pelo `onTrimMemory` e aplica a
    /// política de mídia do PR36. Roda na thread normal do Papo, nunca na do
    /// Java, e cobre ativo e guardados de uma vez.
    #[cfg(target_os = "android")]
    fn pump_memory_pressure(&mut self, ctx: &egui::Context) {
        let Some(level) = crate::platform::memory_pressure::take_pending() else {
            return;
        };
        self.trim_all_media(level);
        // As texturas que saíram ainda estavam na tela deste quadro; o próximo
        // desenha o estado vazio e reconstrói o que estiver visível.
        ctx.request_repaint();
    }

    /// Sonda páginas Turso sem bloquear o frame. O SQL roda no worker do
    /// ClientDb; receivers vazios pedem apenas outro repaint curto.
    fn pump_cache(&mut self, ctx: &egui::Context) {
        use std::sync::mpsc::TryRecvError;

        for workspace in &mut self.workspaces {
            let mut finished = Vec::new();
            for (index, request) in workspace.cache_pages.iter().enumerate() {
                match request.receiver.try_recv() {
                    Ok(result) => finished.push((index, result)),
                    Err(TryRecvError::Empty) => {
                        ctx.request_repaint_after(std::time::Duration::from_millis(16));
                    }
                    Err(TryRecvError::Disconnected) => finished.push((
                        index,
                        Err("worker do cache encerrou antes de devolver a página".to_owned()),
                    )),
                }
            }

            for (index, result) in finished.into_iter().rev() {
                let request = workspace.cache_pages.remove(index);
                if request.restore_epoch != workspace.runtime.store.cache_restore_epoch() {
                    log::debug!(
                        "cache: discarded stale hydration server={} channel={}",
                        workspace.runtime.server_key,
                        request.channel_id
                    );
                    continue;
                }
                match result {
                    Ok(page) => workspace
                        .runtime
                        .store
                        .restore_cached_page(page, request.older),
                    Err(error) => {
                        workspace
                            .runtime
                            .store
                            .cached_page_failed(&request.channel_id, request.older);
                        log::warn!(
                            "cache: hydration failed server={} channel={}: {error}",
                            workspace.runtime.server_key,
                            request.channel_id
                        );
                    }
                }
            }
        }
    }

    /// Lê o que chegou de cada servidor. Todos são atendidos no mesmo
    /// quadro: um servidor que não está na tela ainda precisa contar as
    /// menções e disparar a notificação.
    fn pump_network(&mut self, ctx: &egui::Context) {
        // No modo demonstração a rede não manda no estado.
        if self.demo {
            for ws in &self.workspaces {
                while ws.runtime.net.try_recv().is_some() {}
            }
            return;
        }

        for index in 0..self.workspaces.len() {
            while let Some(effects) = self.workspaces[index].runtime.try_drain() {
                for effect in effects {
                    let ws = &mut self.workspaces[index];
                    match effect {
                        RuntimeEffect::ServerUnlocked => {
                            // O formulário continua sendo apresentação; o
                            // runtime só avisa que a senha do servidor abriu.
                            let username = ws.form.username.trim().to_owned();
                            let password = ws.form.password.clone();
                            if !username.is_empty() && !password.is_empty() {
                                ws.runtime.net.send(Command::Login { username, password });
                            }
                        }
                        RuntimeEffect::ServerLabelChanged(name) => {
                            if ws.label != name {
                                ws.label = name.clone();
                                self.settings.servers[index].label = name;
                            }
                        }
                        RuntimeEffect::OutgoingRejected {
                            owner_user_id,
                            channel_id,
                            content,
                            reply_to,
                            notify_reply,
                        } => {
                            let (visible, bindings) =
                                ws.runtime.store.display_mentions_with_bindings(&content);
                            let draft = crate::ui::shell::DraftState {
                                text: visible,
                                mentions: bindings,
                                reply_to,
                                notify_reply,
                            };
                            let server_key = ws.runtime.server_key.clone();
                            if ws.runtime.cached_owner() != Some(owner_user_id.as_str()) {
                                if index == self.active && self.ui.last_channel == channel_id {
                                    self.ui.composer = draft.text;
                                    self.ui.composer_mentions = draft.mentions;
                                    self.ui.replying = draft.reply_to;
                                    self.ui.reply_notify = draft.notify_reply;
                                }
                                continue;
                            }
                            if index == self.active {
                                if self.ui.drafts.owner() != Some(owner_user_id.as_str()) {
                                    self.ui.drafts.reset_owner(&owner_user_id);
                                }
                                self.ui.drafts.put(&channel_id, draft.clone());
                                if self.ui.last_channel == channel_id {
                                    self.ui.composer = draft.text.clone();
                                    self.ui.composer_mentions = draft.mentions.clone();
                                    self.ui.replying = draft.reply_to.clone();
                                    self.ui.reply_notify = draft.notify_reply;
                                }
                                let ops =
                                    self.ui.drafts.take_persistence_ops(&owner_user_id, true);
                                if !ops.is_empty() {
                                    self.cache.submit(&server_key, ops);
                                }
                            } else {
                                if ws.stash.drafts.owner() != Some(owner_user_id.as_str()) {
                                    ws.stash.drafts.reset_owner(&owner_user_id);
                                }
                                ws.stash.drafts.put(&channel_id, draft.clone());
                                if ws.stash.last_channel == channel_id {
                                    ws.stash.composer = draft.text;
                                    ws.stash.composer_mentions = draft.mentions;
                                    ws.stash.replying = draft.reply_to;
                                    ws.stash.reply_notify = draft.notify_reply;
                                }
                                let ops =
                                    ws.stash.drafts.take_persistence_ops(&owner_user_id, true);
                                if !ops.is_empty() {
                                    self.cache.submit(&server_key, ops);
                                }
                            }
                        }
                        RuntimeEffect::Call(effect) => route_call_effect(ws, *effect, ctx),
                        RuntimeEffect::PushDevice { token, registered } => {
                            let key = ws.runtime.server_key.clone();
                            if registered {
                                self.settings.push_devices.insert(key, token);
                            } else if self.settings.push_devices.get(&key) == Some(&token) {
                                self.settings.push_devices.remove(&key);
                            }
                        }
                        RuntimeEffect::PasswordResetLink { url, expires_at } => {
                            crate::platform::copy::text(ctx, url.clone());
                            let when = expires_at.with_timezone(&chrono::Local)
                                .format("%Y-%m-%d %H:%M")
                                .to_string();
                            self.ui.error = Some((
                                format!("Link de reset copiado · expira em {when}"),
                                ctx.input(|input| input.time),
                            ));
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "android")]
    fn handle_android_notification_navigation(&mut self, ctx: &egui::Context) {
        let Some(target) = crate::platform::android_message::take_navigation() else {
            return;
        };
        let index = if target.server_url.is_empty() {
            // Notificação do FCM desenhada pelo próprio sistema: o toque só
            // traz o canal, e o servidor é o que o conhece.
            let found = self.workspaces.iter().position(|ws| {
                ws.runtime
                    .store
                    .channels
                    .iter()
                    .any(|channel| channel.id == target.channel_id)
            });
            let Some(index) = found else {
                let loading = self.workspaces.iter().any(|ws| {
                    let store = &ws.runtime.store;
                    store.screen == Screen::Starting
                        || (store.screen == Screen::Chat && store.channels.is_empty())
                });
                if loading {
                    crate::platform::android_message::defer_navigation(target);
                }
                return;
            };
            index
        } else {
            let wanted = normalise_server_url(&target.server_url);
            let Some(index) = self
                .workspaces
                .iter()
                .position(|ws| normalise_server_url(&ws.runtime.url) == wanted)
            else {
                return;
            };
            index
        };

        let ready = self.workspaces[index].runtime
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
        ws.runtime.store.selected_channel = target.channel_id;
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
    /// No Android não existe: a call fica no overlay dentro da Activity.
    #[cfg(not(target_os = "android"))]
    fn call_window(&mut self, ctx: &egui::Context) {
        let active = self.active;
        if !self.workspaces[active].runtime.store.call.popped_out {
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
                            &ws.runtime.store,
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
            ws.runtime.store.call.popped_out = false;
        }
    }

    /// Trilho de servidores e o que ele pediu.
    /// As entradas do trilho, com o ícone de cada servidor já em textura.
    fn rail_entries(&mut self, ctx: &egui::Context) -> Vec<crate::ui::rail::Entry> {
        let mut entries = Vec::with_capacity(self.workspaces.len());
        for ws in &self.workspaces {
            let mut entry = ws.entry(&self.settings);
            entry.icon = ws
                .runtime
                .store
                .server
                .as_ref()
                .and_then(|server| server.icon.as_deref())
                .and_then(|blob| self.ui.media.server_icon(blob))
                .and_then(|texture| texture.frame(ctx))
                .map(|handle| handle.id());
            entries.push(entry);
        }
        entries
    }

    fn draw_rail(&mut self, ui: &mut egui::Ui, s: &'static crate::i18n::Strings, ctx: &egui::Context) {
        let entries = self.rail_entries(ctx);
        let direct_unread = self.workspaces[self.active].runtime.store.direct_unread_total();
        if let Some(action) = crate::ui::rail::draw(
            ui,
            &entries,
            self.active,
            direct_unread,
            self.ui.dm_surface,
            &self.tokens,
            s,
        ) {
            self.handle_rail_action(action, ctx);
        }
    }

    fn handle_rail_action(&mut self, action: crate::ui::rail::RailAction, ctx: &egui::Context) {
        match action {
            crate::ui::rail::RailAction::Select(index) => {
                self.add_server_previous = None;
                self.activate(index, ctx);
                self.ui.dm_surface = false;
                let store = &mut self.workspaces[index].runtime.store;
                if store.channel(&store.selected_channel).is_some_and(|channel| channel.kind == crate::state::ChannelKind::Direct)
                    || store.selected_channel.is_empty()
                {
                    let remembered = self.ui.last_server_channel.clone();
                    if store
                        .channel(&remembered)
                        .is_some_and(|channel| channel.kind != crate::state::ChannelKind::Direct)
                    {
                        store.selected_channel = remembered;
                    } else if let Some(channel) = store
                        .channels
                        .iter()
                        .find(|channel| channel.kind == crate::state::ChannelKind::Text)
                    {
                        store.selected_channel = channel.id.clone();
                    }
                }
                self.ui.mobile_surface = crate::ui::shell::MobileSurface::Chat;
            }
            crate::ui::rail::RailAction::DirectMessages => {
                let store = &mut self.workspaces[self.active].runtime.store;
                if !self.ui.dm_surface
                    && store.channel(&store.selected_channel).is_some_and(|channel| channel.kind != crate::state::ChannelKind::Direct)
                {
                    self.ui.last_server_channel = store.selected_channel.clone();
                }
                self.ui.dm_surface = true;
                if !store.channel(&store.selected_channel).is_some_and(|channel| channel.kind == crate::state::ChannelKind::Direct) {
                    store.selected_channel = store
                        .direct_messages
                        .first()
                        .map(|dm| dm.id.clone())
                        .unwrap_or_default();
                }
                self.ui.mobile_surface = crate::ui::shell::MobileSurface::Chat;
            }
            crate::ui::rail::RailAction::Add => self.add_server(ctx),
            crate::ui::rail::RailAction::Remove(index) => self.remove_server(index, ctx),
        }
    }

    /// Respostas dos diálogos do sistema e arquivos soltos na janela.
    fn pump_files(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "android")]
        self.ui
            .attachments
            .extend(crate::platform::files::take_clipboard_attachments());

        for chosen in self.dialogs.poll() {
            match chosen {
                Chosen::Files(uploads) => self.ui.attachments.extend(uploads),
                Chosen::Folder(path) => self.settings.downloads = DownloadMode::Folder(path),
                Chosen::SaveAs { id, name, dest } => self.ui.media.save(&id, &name, dest),
                Chosen::SaveCached { source, dest } => {
                    if let Some(parent) = dest.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    if let Err(error) = std::fs::copy(source, dest) {
                        log::warn!("não deu para salvar imagem de link: {error}");
                    }
                }
                Chosen::Crop {
                    purpose,
                    path,
                    name,
                    size,
                    preview,
                    frames,
                } => {
                    self.crop = Some(crate::ui::crop::CropEditor::new(
                        ctx, purpose, path, name, size, preview, frames,
                    ));
                }
                Chosen::Unreadable => {
                    let s = self.settings.lang.strings();
                    self.ui.error = Some((s.image_unreadable.to_owned(), ctx.input(|input| input.time)));
                }
                Chosen::Image {
                    purpose,
                    blob,
                    format,
                    shrunk: _,
                    name,
                } => match purpose {
                    ImagePick::Avatar => {
                        self.workspaces[self.active].runtime
                            .net
                            .send(Command::SetAvatar { blob, format });
                    }
                    ImagePick::Banner => {
                        self.workspaces[self.active].runtime
                            .net
                            .send(Command::SetBanner { blob, format });
                    }
                    ImagePick::ServerIcon => {
                        let ws = &self.workspaces[self.active];
                        ws.runtime.net.send(Command::PatchServer(Box::new(
                            crate::api::models::PatchServerRequest {
                                icon_blob: Some(blob),
                                icon_format: Some(format),
                                ..Default::default()
                            },
                        )));
                    }
                    // A arte da atividade manual fica neste aparelho, ao lado
                    // dos outros ajustes locais do Rich Presence.
                    ImagePick::ActivityArt => self.store_activity_art(&blob, &format),
                    // O nome veio do editor: a figurinha sobe direto.
                    ImagePick::Sticker => {
                        if let Some(name) = name.filter(|name| !name.is_empty()) {
                            self.workspaces[self.active]
                                .runtime
                                .net
                                .send(Command::CreateEmoji { name, blob, format });
                        }
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
        let model = build_menu(&self.settings, &self.workspaces[self.active].runtime.store);
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
        let height = screen.height() as i32;
        let mut regions = vec![(0, 0, left as i32, height)];
        if self.settings.show_members {
            let width = crate::ui::shell::MEMBERS_WIDTH as i32;
            regions.push((screen.width() as i32 - width, 0, width, height));
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
                #[cfg(any(target_os = "linux", target_os = "windows"))]
                if self.settings.close_to_tray && self.tray.is_some() {
                    self.hide_window(ctx);
                    return;
                }
                self.quit(ctx);
            }
            MenuCommand::SignOut => {
                // O token sai antes da sessão: é ela que autoriza o DELETE.
                let key = self.ws().runtime.server_key.clone();
                if let Some(token) = self.settings.push_devices.get(&key).cloned() {
                    self.ws().runtime.net.send(Command::UnregisterPushDevice { token });
                }
                self.ws().runtime.net.send(Command::Logout);
            }
            MenuCommand::Preferences => self
                .sheet
                .toggle(crate::ui::settings::Surface::App),
            MenuCommand::SwitchLanguage(lang) => self.settings.lang = lang,
            MenuCommand::Quit => self.quit(ctx),
            // Marcar tudo como lido limpa todos os servidores: é o que o
            // contador da bandeja está somando.
            MenuCommand::MarkAllRead => {
                for ws in &mut self.workspaces {
                    ws.runtime.store.mark_all_read();
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
            MenuCommand::Roles => {
                let ws = &self.workspaces[self.active];
                if !ws.runtime.store.can_manage_roles() {
                    return;
                }
                self.sheet.open = Some(crate::ui::settings::Surface::Server);
                self.sheet.server_pane = crate::ui::settings::ServerPane::Roles;
                self.sheet.opened_by_click = true;
                ws.runtime.net.send(Command::LoadRoles);
            }
            MenuCommand::ServerSettings => {
                let ws = &self.workspaces[self.active];
                if self.sheet.open_server_admin(&ws.runtime.store) {
                    ws.runtime.net.send(Command::LoadRoles);
                    if ws.runtime.store.can_manage_server() {
                        ws.runtime.net.send(Command::LoadAuditLogs(Default::default()));
                    }
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
            AdminAction::PickBanner => {
                self.dialogs.pick_image(ctx.clone(), ImagePick::Banner);
                return;
            }
            AdminAction::PickServerIcon => {
                self.dialogs.pick_image(ctx.clone(), ImagePick::ServerIcon);
                return;
            }
            // Vazios: o contrato remove o banner.
            AdminAction::RemoveBanner => Command::SetBanner {
                blob: String::new(),
                format: String::new(),
            },
            AdminAction::SaveProfile(request) => Command::UpdateProfile(request),
            AdminAction::SetPresence(status) => Command::SetStatus { status },
            AdminAction::ChangePassword(password) => Command::ChangePassword { password },
            AdminAction::LoadDevices => Command::LoadDevices,
            AdminAction::DropConnection(connection_id) => Command::DropConnection { connection_id },
            AdminAction::SaveServer(request) => Command::PatchServer(request),
            AdminAction::CreateSticker { name, blob, format } => {
                Command::CreateEmoji { name, blob, format }
            }
            AdminAction::DeleteEmoji(emoji_id) => Command::DeleteEmoji { emoji_id },
            AdminAction::LoadAuditLogs(query) => Command::LoadAuditLogs(query),
        };
        self.workspaces[self.active].runtime.net.send(command);
    }

    fn handle_role(&mut self, action: crate::ui::roles::RoleAction) {
        use crate::ui::roles::RoleAction;

        // Na demonstração não há servidor: dar, tirar e editar cargo mudam
        // o estado aqui mesmo, para a tela de Cargos responder.
        if self.demo {
            let store = &mut self.workspaces[self.active].runtime.store;
            match action {
                RoleAction::Assign { user_id, role_id } => {
                    if let Some(member) = store.members.iter_mut().find(|m| m.id == user_id)
                        && !member.roles.contains(&role_id)
                    {
                        member.roles.push(role_id);
                    }
                }
                RoleAction::Unassign { user_id, role_id } => {
                    if let Some(member) = store.members.iter_mut().find(|m| m.id == user_id) {
                        member.roles.retain(|id| id != &role_id);
                    }
                }
                RoleAction::Update { role_id, name, color, permissions } => {
                    if let Some(role) = store.roles.iter_mut().find(|r| r.id == role_id) {
                        role.name = name;
                        role.color = color;
                        role.permissions = permissions;
                    }
                }
                RoleAction::Create { .. } | RoleAction::Delete(_) => {}
            }
            return;
        }

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
        self.workspaces[self.active].runtime.net.send(command);
    }

    /// A folha de ajustes, ancorada na pastilha que a abriu. É a única
    /// tela de ajustes que existe: conta, aparência, avisos, arquivos,
    /// idioma e sessões de um lado; servidor, canais, cargos, figurinhas e
    /// auditoria do outro.
    /// Editor de recorte, por cima de tudo, até salvar ou cancelar.
    fn crop_editor(&mut self, ctx: &egui::Context) {
        let Some(editor) = &mut self.crop else {
            return;
        };
        let s = self.settings.lang.strings();
        let t = self.tokens;
        let store = &self.workspaces[self.active].runtime.store;
        let me = store.member(&store.me);
        let face = crate::ui::crop::Face {
            name: store.my_name.clone(),
            texture: self
                .ui
                .media
                .avatar(&store.me, store.avatars.get(&store.me).map(String::as_str))
                .and_then(|texture| texture.frame(ctx))
                .map(|handle| handle.id()),
            initials: me.map(|member| member.initials()).unwrap_or_default(),
            tint: me
                .and_then(|member| member.role_color)
                .map(|[r, g, b]| egui::Color32::from_rgb(r, g, b))
                .unwrap_or(t.accent),
        };
        let back = std::mem::take(&mut self.crop_back);
        match crate::ui::crop::draw(ctx, editor, &t, s, &face, back) {
            Some(crate::ui::crop::CropOutcome::Save(crop, name)) => {
                let purpose = editor.purpose;
                let path = editor.path.clone();
                self.dialogs.prepare_crop(ctx.clone(), purpose, path, crop, name);
                self.crop = None;
            }
            Some(crate::ui::crop::CropOutcome::Cancel) => self.crop = None,
            None => {}
        }
    }

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
        let before_rich_presence = self.settings.rich_presence.clone();
        // O início automático é estado do sistema, não um ajuste guardado: o
        // que vale é o arquivo em disco, então ele é lido e escrito à parte.
        let autostart_before = crate::platform::autostart::is_enabled();
        let mut autostart = autostart_before;
        let before_portable = (self.settings.theme, self.settings.notifications);
        let before_primary = (
            self.settings.lang,
            self.settings.theme,
            self.settings.translucency,
            self.settings.show_members,
            self.settings.notifications,
            self.settings.reply_notifications,
            self.settings.close_to_tray,
            self.settings.badge,
        );
        let before_secondary = (
            self.settings.topic_reveal,
            self.settings.open_at_newest,
            self.settings.record_button,
            self.settings.channel_emoji_monochrome,
            self.settings.self_card,
            self.settings.webembed_offscreen,
            self.settings.webembed_scope,
            ask_download,
        );

        let diagnostics: Vec<crate::ui::settings::WorkspaceDiagnostics> = self
            .workspaces
            .iter()
            .map(|workspace| crate::ui::settings::WorkspaceDiagnostics {
                label: workspace.label.clone(),
                server_key: crate::state::server_key(&workspace.runtime.url),
                runtime: workspace.runtime.net.diagnostics(),
                store: workspace.runtime.store.diagnostics(),
                cache_enabled: workspace.runtime.cache.is_enabled(),
                cache: workspace.runtime.cache.stats(),
                notification: self.notification.diagnostics(&workspace.runtime.server_key),
            })
            .collect();

        let preview_diagnostics = self
            .ui
            .previews
            .as_ref()
            .map(|preview| preview.stats())
            .unwrap_or_default();
        let actions = {
            let ws = &self.workspaces[self.active];
            let mut data = crate::ui::settings::Context {
                store: &ws.runtime.store,
                server_url: &ws.runtime.url,
                media: &mut self.ui.media,
                collapsed: &mut self.ui.collapsed_categories,
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
                open_at_newest: &mut self.settings.open_at_newest,
                record_button: &mut self.settings.record_button,
                channel_emoji_monochrome: &mut self.settings.channel_emoji_monochrome,
                self_card: &mut self.settings.self_card,
                webembed_offscreen: &mut self.settings.webembed_offscreen,
                webembed_scope: &mut self.settings.webembed_scope,
                rich_presence: &mut self.settings.rich_presence,
                rich_presence_snapshot: self.rich_presence.snapshot(),
                ask_download: &mut ask_download,
                download_dir: match &self.settings.downloads {
                    DownloadMode::Folder(dir) => Some(dir.display().to_string()),
                    DownloadMode::Ask => None,
                },
                diagnostics: &diagnostics,
                preview: preview_diagnostics,
                #[cfg(any(target_os = "windows", target_os = "android"))]
                update_status: self.update_status.as_deref(),
                #[cfg(any(target_os = "windows", target_os = "android"))]
                update_enabled: self.updater.enabled(),
            };
            crate::ui::settings::sheet(ctx, &mut self.sheet, &mut data, anchor, screen, &t, s)
        };

        // Um ajuste local mudou: grava e reflete na janela na hora.
        let after_primary = (
            self.settings.lang,
            self.settings.theme,
            self.settings.translucency,
            self.settings.show_members,
            self.settings.notifications,
            self.settings.reply_notifications,
            self.settings.close_to_tray,
            self.settings.badge,
        );
        let after_secondary = (
            self.settings.topic_reveal,
            self.settings.open_at_newest,
            self.settings.record_button,
            self.settings.channel_emoji_monochrome,
            self.settings.self_card,
            self.settings.webembed_offscreen,
            self.settings.webembed_scope,
            ask_download,
        );
        if before_primary != after_primary || before_secondary != after_secondary {
            self.ui.channel_emoji_monochrome = self.settings.channel_emoji_monochrome;
            self.ui.self_card = self.settings.self_card;
            self.ui.open_at_newest = self.settings.open_at_newest;
            self.ui.show_record = self.settings.record_button;
            if ask_download != before_secondary.7 {
                self.settings.downloads = if ask_download {
                    DownloadMode::Ask
                } else {
                    DownloadMode::Folder(files::downloads_dir())
                };
            }
            self.retheme(ctx);
        }

        let after_portable = (self.settings.theme, self.settings.notifications);
        if before_portable != after_portable {
            self.queue_portable_settings();
        }

        if before_rich_presence != self.settings.rich_presence {
            self.rich_presence
                .configure(self.settings.rich_presence.clone());
        }

        if autostart != autostart_before
            && let Err(error) = crate::platform::autostart::set(autostart)
        {
            log::warn!("não deu para ajustar o início automático: {error}");
        }

        for action in actions {
            match action {
                SettingsAction::Menu(command) => self.ui.pending.push(command),
                SettingsAction::RestartRichPresence => self.rich_presence.restart(),
                SettingsAction::PickActivityImage => {
                    self.dialogs.pick_image(ctx.clone(), ImagePick::ActivityArt);
                }
                SettingsAction::ClearActivityImage => {
                    if let Some(path) = self.settings.rich_presence.override_activity.image.take() {
                        let _ = std::fs::remove_file(path);
                    }
                    self.rich_presence
                        .configure(self.settings.rich_presence.clone());
                }
                SettingsAction::PickDownloadFolder => {
                    let start = match &self.settings.downloads {
                        DownloadMode::Folder(dir) => dir.clone(),
                        DownloadMode::Ask => files::downloads_dir(),
                    };
                    self.dialogs.pick_folder(ctx.clone(), start);
                }
                #[cfg(any(target_os = "windows", target_os = "android"))]
                SettingsAction::CheckUpdates => {
                    self.update_status = Some(self.settings.lang.strings().update_checking.to_owned());
                    self.updater.check();
                }
                SettingsAction::Chat(chat) => self.handle_chat(ctx, chat),
                SettingsAction::Admin(admin) => self.handle_admin(ctx, admin),
                SettingsAction::Role(role) => self.handle_role(role),
            }
        }
    }


 }

impl PapoApp {
    /// Grava a arte recortada da atividade manual e passa a usá-la. Cada
    /// escolha ganha um nome novo: o cartão guarda a textura pelo caminho, e
    /// trocar o arquivo por baixo do mesmo nome mostraria a imagem antiga.
    fn store_activity_art(&mut self, blob: &str, format: &str) {
        use base64::Engine as _;
        let bytes = match base64::engine::general_purpose::STANDARD.decode(blob) {
            Ok(bytes) => bytes,
            Err(error) => {
                log::warn!("arte da atividade inválida: {error}");
                return;
            }
        };
        let Some(dir) = crate::platform::dirs::data_dir().map(|dir| dir.join("rich-presence")) else {
            return;
        };
        let stamp = chrono::Utc::now().timestamp_millis();
        let path = dir.join(format!("activity-{stamp}.{}", format.to_ascii_lowercase()));
        if let Err(error) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, bytes)) {
            log::warn!("não deu para gravar a arte da atividade: {error}");
            return;
        }
        let manual = &mut self.settings.rich_presence.override_activity;
        if let Some(old) = manual.image.replace(path) {
            let _ = std::fs::remove_file(old);
        }
        self.rich_presence
            .configure(self.settings.rich_presence.clone());
    }

    fn project_presence_activity(&mut self, ctx: &egui::Context) {
        let interacted = ctx.input(|input| {
            !input.events.is_empty()
                || !input.keys_down.is_empty()
                || input.pointer.delta() != egui::Vec2::ZERO
                || input.pointer.any_pressed()
        });
        if !interacted {
            return;
        }

        let now = std::time::Instant::now();
        for workspace in &mut self.workspaces {
            if workspace.runtime.store.screen != Screen::Chat
                || !matches!(
                    workspace.runtime.store.connection,
                    crate::api::ws::Connection::Online
                )
            {
                continue;
            }
            let due = workspace
                .last_presence_activity
                .is_none_or(|last| now.duration_since(last) >= std::time::Duration::from_secs(20));
            if due {
                workspace.last_presence_activity = Some(now);
                workspace.runtime.net.send(Command::PresenceActivity);
            }
        }
    }

    fn project_local_activity(&mut self) {
        let activity = self.rich_presence.snapshot().activity.clone();
        for workspace in &mut self.workspaces {
            if workspace.published_activity.as_ref() != Some(&activity) {
                workspace.published_activity = Some(activity.clone());
                workspace
                    .runtime
                    .net
                    .send(Command::SetActivity(activity.as_ref().map(activity_for_server)));
            }

            let me = workspace.runtime.store.me.clone();
            if me.is_empty() {
                continue;
            }
            match activity.clone() {
                Some(activity) => {
                    workspace.runtime.store.activities.insert(me, activity);
                }
                None => {
                    workspace.runtime.store.activities.remove(&me);
                }
            }
        }
    }

    fn ensure_active_drafts_loaded(&mut self) {
        let index = self.active;
        let owner = self.workspaces[index]
            .runtime
            .cached_owner()
            .unwrap_or_default()
            .to_owned();
        if owner.is_empty() {
            if self.ui.drafts.owner().is_some() {
                self.ui.drafts = Default::default();
                self.ui.composer.clear();
                self.ui.composer_mentions.clear();
                self.ui.replying = None;
                self.ui.reply_notify = self.ui.reply_notify_default;
            }
            return;
        }
        if self.ui.drafts.is_loaded_for(&owner) {
            return;
        }

        let server_key = self.workspaces[index].runtime.server_key.clone();
        let channel_id = self.ui.last_channel.clone();
        let provisional = (self.ui.drafts.owner().is_none())
            .then(|| self.ui.draft_state())
            .filter(|draft| !draft.is_empty());

        match self.cache.load_drafts(&server_key, &owner) {
            Ok(drafts) => {
                let rows = drafts.len();
                self.ui.drafts.load(&owner, drafts);
                log::debug!("draft restored server={server_key} rows={rows}");
            }
            Err(error) => {
                self.ui.drafts.load(&owner, Vec::new());
                log::warn!("draft persistence failed server={server_key}: {error}");
            }
        }

        if !channel_id.is_empty() {
            self.ui.restore_draft(&channel_id);
            if let Some(provisional) = provisional {
                self.ui.drafts.put(&channel_id, provisional.clone());
                self.ui.composer = provisional.text;
                self.ui.composer_mentions = provisional.mentions;
                self.ui.replying = provisional.reply_to;
                self.ui.reply_notify = provisional.notify_reply;
            }
        }
    }

    fn persist_active_drafts(&mut self, force: bool) {
        if self.workspaces.is_empty() {
            return;
        }
        let owner = self.workspaces[self.active]
            .runtime
            .cached_owner()
            .unwrap_or_default()
            .to_owned();
        if owner.is_empty() {
            return;
        }
        let channel_id = self.ui.last_channel.clone();
        if !channel_id.is_empty() {
            self.ui.capture_draft(&channel_id);
        }
        let ops = self.ui.drafts.take_persistence_ops(&owner, force);
        if !ops.is_empty() {
            let server_key = self.workspaces[self.active].runtime.server_key.clone();
            self.cache.submit(&server_key, ops);
        }
    }

    fn flush_all_drafts(&mut self) {
        self.persist_active_drafts(true);
        for (index, workspace) in self.workspaces.iter_mut().enumerate() {
            if index == self.active {
                continue;
            }
            let owner = workspace.runtime.cached_owner().unwrap_or_default().to_owned();
            if owner.is_empty() {
                continue;
            }
            let ops = workspace.stash.drafts.take_persistence_ops(&owner, true);
            if !ops.is_empty() {
                self.cache.submit(&workspace.runtime.server_key, ops);
            }
        }
        self.cache.flush();
    }

    fn apply_portable_config(&mut self, ctx: &egui::Context, config: &crate::api::models::UserConfig) {
        let theme = match config.theme.as_str() {
            "dark" => ThemePref::Dark,
            "light" => ThemePref::Light,
            _ => ThemePref::System,
        };
        let changed = self.settings.theme != theme
            || self.settings.notifications != config.notifications.enabled;
        self.settings.theme = theme;
        self.settings.notifications = config.notifications.enabled;
        if changed {
            self.retheme(ctx);
            #[cfg(target_os = "android")]
            if self.settings.notifications {
                crate::platform::android_message::ensure_permission();
            }
        }
    }

    fn portable_config_from_local(
        &self,
        mut config: crate::api::models::UserConfig,
    ) -> crate::api::models::UserConfig {
        config.theme = match self.settings.theme {
            ThemePref::Dark => "dark",
            ThemePref::Light => "light",
            ThemePref::System => "system",
        }
        .to_owned();
        config.notifications.enabled = self.settings.notifications;
        config
    }

    fn queue_portable_settings(&mut self) {
        if self.demo || self.workspaces.is_empty() {
            return;
        }
        let index = self.active;
        let key = self.workspaces[index].runtime.server_key.clone();
        let base = self.workspaces[index]
            .runtime
            .store
            .user_settings
            .as_ref()
            .map(|settings| settings.config.clone())
            .or_else(|| self.settings.cached_user_settings.get(&key).cloned());
        let Some(base) = base else {
            return;
        };
        let config = self.portable_config_from_local(base);
        self.settings
            .pending_user_settings
            .insert(key, config.clone());
        self.workspaces[index].applied_user_config = Some(config);
        self.workspaces[index].sent_user_config = None;
    }

    /// Converge preferências portáteis do servidor ativo. Um pending local
    /// vence o snapshot remoto até o PUT ser confirmado; sem pending, whoami
    /// é a autoridade quando a conta/servidor muda.
    fn sync_active_portable_settings(&mut self, ctx: &egui::Context) {
        if self.demo || self.workspaces.is_empty() {
            return;
        }
        let index = self.active;
        let key = self.workspaces[index].runtime.server_key.clone();
        let online = matches!(
            self.workspaces[index].runtime.store.connection,
            crate::api::ws::Connection::Online
        );
        if !online {
            self.workspaces[index].sent_user_config = None;
        }

        let remote = self.workspaces[index]
            .runtime
            .store
            .user_settings
            .as_ref()
            .map(|settings| settings.config.clone());
        if let Some(remote) = remote.as_ref() {
            self.settings
                .cached_user_settings
                .insert(key.clone(), remote.clone());
        }

        if let Some(pending) = self.settings.pending_user_settings.get(&key).cloned() {
            if remote.as_ref() == Some(&pending) {
                self.settings.pending_user_settings.remove(&key);
                self.workspaces[index].sent_user_config = None;
                if self.workspaces[index].applied_user_config.as_ref() != Some(&pending) {
                    self.apply_portable_config(ctx, &pending);
                    self.workspaces[index].applied_user_config = Some(pending);
                }
                return;
            }

            if self.workspaces[index].applied_user_config.as_ref() != Some(&pending) {
                self.apply_portable_config(ctx, &pending);
                self.workspaces[index].applied_user_config = Some(pending.clone());
            }
            if online && self.workspaces[index].sent_user_config.as_ref() != Some(&pending) {
                self.workspaces[index]
                    .runtime
                    .net
                    .send(Command::UpdateUserSettings(Box::new(pending.clone())));
                self.workspaces[index].sent_user_config = Some(pending);
            }
            return;
        }

        let effective = remote.or_else(|| self.settings.cached_user_settings.get(&key).cloned());
        if let Some(effective) = effective
            && self.workspaces[index].applied_user_config.as_ref() != Some(&effective)
        {
            self.apply_portable_config(ctx, &effective);
            self.workspaces[index].applied_user_config = Some(effective);
            self.workspaces[index].sent_user_config = None;
        }
    }

    /// Leva o token FCM a cada servidor com sessão e notificações ligadas, e
    /// o tira dos que as desligaram. Sem token (APK sem Firebase, aparelho
    /// sem Play Services) nada acontece e a reconciliação periódica segue.
    #[cfg(target_os = "android")]
    fn sync_push_devices(&mut self) {
        let Some(device) = crate::platform::android_push::device() else {
            return;
        };
        for index in 0..self.workspaces.len() {
            let enabled = self.notification_enabled_for(index);
            let ws = &mut self.workspaces[index];
            let key = &ws.runtime.server_key;
            match ws.runtime.store.screen {
                Screen::Chat if !ws.runtime.store.me.is_empty() => {}
                // A sessão acabou e levou o registro junto.
                Screen::Auth => {
                    self.settings.push_devices.remove(key);
                    ws.push_sent = None;
                    continue;
                }
                _ => continue,
            }
            let desired = (device.token.clone(), enabled);
            if ws.push_sent.as_ref() == Some(&desired) {
                continue;
            }
            if enabled {
                ws.runtime.net.send(Command::RegisterPushDevice {
                    token: device.token.clone(),
                    device_name: device.name.clone(),
                });
            } else if let Some(token) = self.settings.push_devices.get(key) {
                ws.runtime.net.send(Command::UnregisterPushDevice {
                    token: token.clone(),
                });
            }
            ws.push_sent = Some(desired);
        }
    }

    fn notification_enabled_for(&self, index: usize) -> bool {
        let workspace = &self.workspaces[index];
        self.settings
            .pending_user_settings
            .get(&workspace.runtime.server_key)
            .map(|config| config.notifications.enabled)
            .or_else(|| {
                workspace
                    .runtime
                    .store
                    .user_settings
                    .as_ref()
                    .map(|settings| settings.config.notifications.enabled)
            })
            .or_else(|| {
                self.settings
                    .cached_user_settings
                    .get(&workspace.runtime.server_key)
                    .map(|config| config.notifications.enabled)
            })
            .unwrap_or(self.settings.notifications)
    }

    fn sync_notification_contexts(&self) {
        for (index, workspace) in self.workspaces.iter().enumerate() {
            workspace.sync_notification_context(
                self.notification_enabled_for(index),
                index == self.active,
                self.settings
                    .push_devices
                    .contains_key(&workspace.runtime.server_key),
            );
        }

        #[cfg(target_os = "android")]
        {
            let draft = self.add_server_previous.map(|_| self.active);
            crate::platform::android_work::sync_periodic(
                self.workspaces
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| Some(*index) != draft)
                    .map(|(index, workspace)| {
                        (
                            workspace.runtime.server_key.as_str(),
                            workspace.runtime.url.as_str(),
                            self.notification_enabled_for(index),
                            self.settings
                                .push_devices
                                .contains_key(&workspace.runtime.server_key),
                        )
                    }),
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
        self.ui.webembed.set_context(&ctx);
        self.ui.webembed.begin_frame();
        self.ui.webembed.pump_events(&ctx);
        self.ui.webembed.prepare_render(frame);
        let external_urls = self.ui.webembed.take_external_urls();
        for url in external_urls {
            self.ui.request_external_url(&ctx, url);
        }
        #[cfg(target_os = "android")]
        {
            crate::platform::native_text::begin_frame();
            crate::platform::native_field::begin_frame();
            if self.settings.notifications {
                crate::platform::android_message::ensure_permission();
            }
            // A pressão de memória entra pelo JNI e é aplicada aqui, na thread
            // normal — não dentro do `shell::draw`.
            self.pump_memory_pressure(&ctx);
        }
        self.attach_window(frame);

        self.rich_presence.pump();
        // Project every frame, not only on activity changes: a workspace may
        // finish authentication after the current activity was already found.
        self.project_local_activity();

        #[cfg(any(target_os = "windows", target_os = "android"))]
        {
            self.pump_updater(&ctx);
            self.project_update_pill();
        }

        #[cfg(target_os = "android")]
        {
            self.handle_android_notification_navigation(&ctx);
            self.sync_push_devices();
        }
        self.sync_notification_contexts();

        // Indo para segundo plano: gravar agora, porque pode não haver um
        // depois. Perder o foco é o último aviso que o aplicativo recebe
        // antes de o sistema poder encerrá-lo sem mais nada.
        #[cfg(target_os = "android")]
        {
            let focused = ctx.input(|input| input.viewport().focused).unwrap_or(true);
            if self.focused && !focused {
                // Indo para segundo plano, este é o **último quadro**: ao
                // esconder a Activity o laço para de desenhar, então um trim
                // publicado depois ficaria preso no latch até a volta — e é
                // justamente atrás que o sistema quer que a gente largue o
                // que é reconstruível. A política é a mesma (Moderate, que
                // preserva quem está tocando), só aplicada antes da suspensão.
                self.ui.webembed.suspend_active();
                self.trim_all_media(crate::media::TrimLevel::Moderate);
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
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        {
            self.handle_window_lifecycle(&ctx);
            self.notification
                .set_foreground(self.focused && !self.minimized);
        }

        self.pump_network(&ctx);
        self.project_presence_activity(&ctx);
        self.pump_cache(&ctx);
        self.sync_active_portable_settings(&ctx);
        self.ensure_active_drafts_loaded();
        self.sync_notification_contexts();

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
                        if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.runtime.store.call.active()) {
                            ws.runtime.store.call.muted = muted;
                        }
                    }
                    crate::platform::android_call::UiAction::Camera(camera) => {
                        if let Some(ws) = self.workspaces.iter_mut().find(|ws| ws.runtime.store.call.active()) {
                            ws.runtime.store.call.camera = camera;
                        }
                    }
                    crate::platform::android_call::UiAction::Hangup => {
                        if let Some(index) = self.workspaces.iter().position(|ws| ws.runtime.store.call.active()) {
                            leave_call(&mut self.workspaces[index]);
                        }
                    }
                }
            }

            let call_index = self.workspaces.iter().position(|ws| ws.runtime.store.call.active());
            let has_video = call_index
                .is_some_and(|index| self.workspaces[index].runtime.store.call.has_video());

            let (muted, camera, members, speaker_name) = if let Some(index) = call_index {
                let ws = &self.workspaces[index];
                let speaker_name = ws.runtime.store.call.speakers.first().and_then(|id| {
                    ws.runtime.store.member(id).map(|member| member.name.clone())
                });
                (
                    ws.runtime.store.call.muted,
                    ws.runtime.store.call.camera,
                    ws.runtime.store.call.members().len(),
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
                self.ui.webembed.suspend_active();
                ctx.request_repaint();
                if let Some(index) = call_index {
                    let strings = self.settings.lang.strings();
                    let ws = &mut self.workspaces[index];
                    crate::ui::call::pip(
                        ui,
                        &ws.runtime.store,
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

        let compact_chat = matches!(
            self.workspaces[self.active].runtime.store.screen,
            Screen::Chat
        ) && crate::ui::shell::is_compact(ctx.content_rect());
        let add_server_modal = self.add_server_previous.is_some();
        if !compact_chat && !add_server_modal {
            self.draw_rail(ui, strings, &ctx);
        }

        let active = self.active;
        match self.workspaces[active].runtime.store.screen {
            Screen::Starting => auth::starting(ui, &self.tokens, strings),
            Screen::Auth => {
                let response = {
                    let ws = &mut self.workspaces[active];
                    auth::sign_in(ui, &mut ws.form, &ws.runtime.store, &self.tokens, strings)
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
                    auth::create_server(ui, &mut ws.form, &ws.runtime.store, &self.tokens, strings)
                };
                if response.action == AuthAction::CreateServer {
                    let ws = &mut self.workspaces[active];
                    ws.runtime.store.busy = true;
                    ws.runtime.net.send(Command::CreateServer {
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
                let mobile_entries = compact_chat.then(|| self.rail_entries(&ctx));
                self.ui.reply_notify_default = self.settings.reply_notifications;
                self.ui.webembed_behavior = self.settings.webembed_offscreen;
                self.ui.webembed_scope = self.settings.webembed_scope;
                self.ui.webembed_float_width = self.settings.webembed_float_width;
                self.ui.webembed_float_pos = self.settings.webembed_float_pos;
                self.ui.trusted_link_hosts = self.settings.trusted_link_hosts.clone();
                self.ui.gif_favourites = self.settings.gif_favourites.clone();
                self.ui.gif_locale = match self.settings.lang {
                    Lang::PtBr => "pt",
                    Lang::En => "en",
                }
                .to_owned();
                // O voltar do sistema tem um dono por quadro, decidido aqui:
                // o editor de recorte, senão os ajustes, senão o que está
                // aberto por cima da conversa (cartões). Espalhar essa
                // decisão fazia um cartão fechado desligar o voltar de outro.
                let back = crate::platform::back::take();
                if self.crop.is_some() {
                    self.crop_back = back;
                } else if back && self.sheet.open.is_some() {
                    self.sheet.back();
                } else {
                    self.ui.back = back;
                }
                self.ui.webembed_blocked = self.sheet.open.is_some()
                    || self.ui.profile.is_some()
                    || self.ui.server_card.is_some();
                self.ui.server_url = self.workspaces[active].runtime.url.clone();
                self.ui.server_count = self.workspaces.len();
                self.ui.channel_emoji_monochrome = self.settings.channel_emoji_monochrome;
                let draft_channel_before = self.ui.last_channel.clone();
                let direct_unread = self.workspaces[active].runtime.store.direct_unread_total();
                let direct_active = self.ui.dm_surface;
                let rail_action = {
                    let ws = &mut self.workspaces[active];
                    shell::draw(
                        ui,
                        &mut ws.runtime.store,
                        &mut self.ui,
                        ws.call.as_mut(),
                        &self.tokens,
                        strings,
                        mobile_entries.as_ref().map(|entries| shell::MobileServers {
                            entries,
                            active,
                            direct_unread,
                            direct_active,
                        }),
                    )
                };
                self.settings.webembed_float_width = self.ui.webembed_float_width;
                self.settings.webembed_float_pos = self.ui.webembed_float_pos;
                self.settings.trusted_link_hosts = self.ui.trusted_link_hosts.clone();
                self.settings.gif_favourites = self.ui.gif_favourites.clone();
                let draft_channel_after = self.ui.last_channel.clone();
                if !draft_channel_after.is_empty() {
                    self.ui.capture_draft(&draft_channel_after);
                }
                self.persist_active_drafts(draft_channel_before != draft_channel_after);
                if let Some(action) = rail_action {
                    self.handle_rail_action(action, &ctx);
                }
                #[cfg(not(target_os = "android"))]
                self.call_window(&ctx);
                self.pump_chat();
                let actions = std::mem::take(&mut self.ui.actions);
                for action in actions {
                    self.handle_chat(&ctx, action);
                }
            }
        }
        #[cfg(any(target_os = "windows", target_os = "android"))]
        {
            if self.workspaces[self.active].runtime.store.screen != Screen::Chat {
                let full = ui.max_rect();
                let _ = shell::update_pill(
                    ui,
                    &mut self.ui,
                    &self.tokens,
                    strings,
                    full,
                    None,
                    None,
                );
            }
            if std::mem::take(&mut self.ui.update_pill_clicked) {
                self.handle_update_pill_click(&ctx);
            }
        }

        // O editor fica por cima da folha; no quadro em que ele fecha (o
        // clique em Salvar/Cancelar) a folha ainda precisa saber disso.
        self.sheet.modal_above = self.crop.is_some();
        self.settings_sheet(&ctx);
        self.crop_editor(&ctx);
        crate::platform::back::intercept(
            self.crop.is_some()
                || self.sheet.open.is_some()
                || self.ui.profile.is_some()
                || self.ui.server_card.is_some(),
        );
        #[cfg(any(target_os = "windows", target_os = "android"))]
        self.update_prompt(&ctx);
        self.pump_files(&ctx);
        #[cfg(not(target_os = "android"))]
        {
            let store = &self.workspaces[self.active].runtime.store;
            let shortcuts = shortcut_commands(&ctx, store);
            self.ui.pending.extend(shortcuts);
        }

        let pending = std::mem::take(&mut self.ui.pending);
        for command in pending {
            self.handle(&ctx, command);
        }
        // Captura seleção/toggles ocorridos durante este próprio quadro; a
        // thread de rede pode consultar o snapshot sem esperar outro repaint.
        self.sync_notification_contexts();

        // O browser é uma superfície nativa: qualquer frame que não declare
        // um dono/retângulo válido precisa suspendê-lo ou destruí-lo.
        self.ui.webembed.end_frame(
            self.settings.webembed_offscreen,
            self.settings.webembed_scope,
            self.sheet.open.is_some(),
        );

        #[cfg(target_os = "android")]
        {
            crate::platform::native_field::end_frame();
            crate::platform::native_text::end_frame();
        }
    }

    #[cfg(target_os = "android")]
    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        self.flush_all_drafts();
        self.ui.webembed.destroy_active();
        if let (Some(gl), Some(glass)) = (gl, &self.ui.glass)
            && let Ok(glass) = glass.lock()
        {
            glass.destroy(gl);
        }
    }

    #[cfg(not(target_os = "android"))]
    fn on_exit(&mut self) {
        self.flush_all_drafts();
        self.ui.webembed.destroy_active();
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.flush_all_drafts();
        // No Android o eframe grava no `suspend` (onPause), antes de o laço de
        // quadros parar — é o último ponto no objeto da aplicação em que dá
        // para soltar mídia reconstruível antes de a Activity sair de cena.
        // Em primeiro plano o autosave costuma não achar nada: o quadro já
        // drenou o latch. Aqui é onde o pedido publicado no onPause vira trim.
        #[cfg(target_os = "android")]
        if let Some(level) = crate::platform::memory_pressure::take_pending() {
            self.trim_all_media(level);
        }

        // As marcas de leitura vivem no estado de cada servidor; só o ajuste
        // persiste, e cada servidor guarda as suas sob a própria chave.
        let draft = self.add_server_previous.map(|_| self.active);
        for (index, ws) in self.workspaces.iter().enumerate() {
            if Some(index) == draft {
                continue;
            }
            self.settings
                .server_marks
                .insert(crate::state::server_key(&ws.runtime.url), ws.runtime.store.read_marks.clone());
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
                url: ws.runtime.url.clone(),
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
/// Os atalhos que o menu anuncia. O menu global só os mostra, e fora do
/// Plasma nem menu há: quem os atende é a própria janela.
#[cfg(not(target_os = "android"))]
fn shortcut_commands(ctx: &egui::Context, store: &Store) -> Vec<MenuCommand> {
    use egui::{Key, KeyboardShortcut, Modifiers};

    let shortcuts = [
        (Key::Comma, MenuCommand::Preferences),
        (Key::Q, MenuCommand::Quit),
        (Key::N, MenuCommand::NewChannel),
        (Key::F, MenuCommand::Search),
        (Key::U, MenuCommand::ToggleMembers),
    ];
    ctx.input_mut(|input| {
        shortcuts
            .into_iter()
            .filter(|(key, _)| {
                input.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, *key))
            })
            .map(|(_, command)| command)
            // O menu desabilita o item para quem não pode criar canais.
            .filter(|command| *command != MenuCommand::NewChannel || store.can_manage_channels())
            .collect()
    })
}

#[cfg(target_os = "linux")]
fn build_menu(settings: &Settings, store: &Store) -> MenuModel {
    let s = settings.lang.strings();
    MenuModel::new(vec![
        MenuNode::submenu(
            s.menu_papo,
            vec![
                MenuNode::item(s.menu_about, MenuCommand::About),
                MenuNode::separator(),
                MenuNode::item(s.menu_preferences, MenuCommand::Preferences)
                    .accel(&["Control", "comma"]),
                MenuNode::item(s.menu_roles, MenuCommand::Roles)
                    .enabled(store.can_manage_roles()),
                MenuNode::item(s.menu_server, MenuCommand::ServerSettings)
                    .enabled(store.can_open_server_admin()),
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
                    .enabled(store.can_manage_channels())
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

#[cfg(any(target_os = "linux", target_os = "windows"))]
fn tray_labels(settings: &Settings) -> TrayLabels {
    let s = settings.lang.strings();
    TrayLabels {
        open: s.tray_open.to_owned(),
        restart_rich_presence: s.rich_presence_restart.to_owned(),
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