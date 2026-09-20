//! Aplicação: junta estado, tema e telas.

use egui::Color32;

use crate::i18n::Lang;
use crate::media::Media;
use crate::platform::files::{self, Chosen, Dialogs, ImagePick};
use crate::platform::menu::{MenuCommand, MenuModel, MenuNode};
use crate::platform::desktop;
#[cfg(target_os = "linux")]
use crate::platform::activate::Activator;
use crate::platform::notify::{Notification, Notifier};
use crate::platform::tray::{Tray, TrayCommand, TrayLabels};
#[cfg(target_os = "linux")]
use crate::platform::launcher::{Badge, Launcher};
use crate::platform::{appmenu::AppMenuSurface, blur::BlurSurface, global_menu::GlobalMenu};
use crate::api::net::{Command, Net};
use crate::state::{Screen, Store};
use crate::ui::auth::{self, AuthAction, AuthForm};
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
        let label = host_of(&url);
        Self { url, label }
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
        if self.servers.is_empty() {
            self.servers.push(ServerEntry::new(self.server_url.clone()));
            // As marcas antigas eram todas do único servidor que existia.
            if !self.read_marks.is_empty() {
                let key = crate::state::server_key(&self.server_url);
                self.server_marks
                    .insert(key, std::mem::take(&mut self.read_marks));
            }
        }
        self.active = self.active.min(self.servers.len() - 1);
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
    pub form: AuthForm,
    /// O pedaço da interface deste servidor, fora enquanto outro está na tela.
    pub stash: shell::Stash,
    /// Instante do último evento de digitação enviado.
    pub typing_sent: Option<std::time::Instant>,
}

impl Workspace {
    fn open(entry: &ServerEntry, marks: &ReadMarks, ctx: &egui::Context) -> Self {
        let net = Net::spawn(entry.url.clone(), ctx.clone());
        // A mídia usa o cookie da sessão deste servidor para baixar anexos.
        let media = Media::spawn(
            entry.url.clone(),
            std::sync::Arc::clone(&net.session),
            ctx.clone(),
        );
        let mut store = Store::default();
        store.read_marks = marks
            .get(&crate::state::server_key(&entry.url))
            .cloned()
            .unwrap_or_default();
        Self {
            url: entry.url.clone(),
            label: entry.label.clone(),
            net,
            store,
            form: AuthForm {
                server_url: entry.url.clone(),
                ..AuthForm::default()
            },
            stash: shell::Stash::new(crate::media::MediaStore::new(media)),
            typing_sent: None,
        }
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

/// Formulário de canal. Sem `id` é criação; com `id`, edição — os dois usam
/// os mesmos campos, e é só o botão final que muda.
#[derive(Clone, Debug, Default)]
struct ChannelDialog {
    id: Option<String>,
    name: String,
    topic: String,
    /// `text`, `voice` ou `category`, como o contrato espera.
    kind: String,
    /// O foco vai para o nome uma vez, ao abrir. Pedi-lo a cada quadro
    /// arrancava o cursor de quem tivesse clicado no tópico, e o que se via
    /// era o campo piscando.
    focus: bool,
}

impl ChannelDialog {
    fn create() -> Self {
        Self {
            kind: "text".to_owned(),
            focus: true,
            ..Default::default()
        }
    }

    fn edit(channel: &crate::state::Channel) -> Self {
        use crate::state::ChannelKind;
        Self {
            id: Some(channel.id.clone()),
            name: channel.name.clone(),
            topic: channel.topic.clone().unwrap_or_default(),
            kind: match channel.kind {
                ChannelKind::Voice => "voice".to_owned(),
                ChannelKind::Category => "category".to_owned(),
                ChannelKind::Text => "text".to_owned(),
            },
            focus: true,
        }
    }
}

pub struct PapoApp {
    workspaces: Vec<Workspace>,
    /// Índice do servidor na tela.
    active: usize,
    ui: UiState,
    settings: Settings,
    system: SystemTheme,
    tokens: Tokens,
    settings_open: bool,
    /// Diálogo de canal aberto: criar (sem id) ou editar (com id).
    channel_dialog: Option<ChannelDialog>,
    /// Busca aberta, com o termo digitado.
    search: Option<String>,
    /// Idem ao diálogo de canal: foco no termo só ao abrir.
    search_focus: bool,
    about_open: bool,
    roles: crate::ui::roles::RolesState,
    admin: crate::ui::admin::AdminState,
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
}

impl PapoApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();
        settings.normalise();

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

        let glass = cc.gl.as_ref().and_then(|gl| GlassRenderer::new(gl));
        if glass.is_none() {
            log::warn!("sem backend glow: o vidro fosco fica desligado");
        }

        // Todos os servidores sobem juntos: o que chega num deles enquanto
        // outro está na tela ainda conta para o contador e a notificação.
        let mut workspaces: Vec<Workspace> = settings
            .servers
            .iter()
            .map(|entry| Workspace::open(entry, &settings.server_marks, &cc.egui_ctx))
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
                    workspaces.push(Workspace::open(&entry, &settings.server_marks, &cc.egui_ctx));
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
        let mut ui_state = UiState {
            show_members: settings.show_members,
            translucent: settings.translucency,
            reveal_topic: settings.topic_reveal,
            show_record: settings.record_button,
            glass,
            ..UiState::default()
        };
        workspaces[active].stash.swap(&mut ui_state);

        Self {
            workspaces,
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
            settings_open: false,
            channel_dialog: None,
            search: None,
            search_focus: false,
            about_open: false,
            roles: Default::default(),
            admin: Default::default(),
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
            own_chrome: !desktop::uses_global_menu(),
            header: crate::ui::headerbar::HeaderState::default(),
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
            body: message.content.clone().unwrap_or_default(),
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
        let url = self.workspaces[index].form.server_url.trim().to_owned();
        if !url.is_empty() && url != self.workspaces[index].url {
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
        let entry = ServerEntry::new(url);
        let mut fresh = Workspace::open(&entry, &self.settings.server_marks, ctx);
        fresh.form = AuthForm {
            server_url: entry.url.clone(),
            ..form
        };
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

    /// Acrescenta um servidor vazio e já o coloca na tela, esperando o
    /// endereço.
    fn add_server(&mut self, ctx: &egui::Context) {
        let entry = ServerEntry::new(default_server_url());
        let workspace = Workspace::open(&entry, &self.settings.server_marks, ctx);
        self.settings.servers.push(entry);
        self.workspaces.push(workspace);
        self.activate(self.workspaces.len() - 1, ctx);
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
        crate::api::net::forget(&self.workspaces[index].url);
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

        if let Some(channel_id) = ws.store.channel_needing_messages() {
            ws.store.mark_loading(&channel_id);
            ws.net.send(Command::LoadMessages {
                channel_id: channel_id.clone(),
            });
            ws.net.send(Command::LoadPinned { channel_id });
        }

        // Com a janela à frente, o canal aberto está sendo lido agora.
        if focused && !ws.store.selected_channel.is_empty() {
            let channel_id = ws.store.selected_channel.clone();
            ws.store.mark_read(&channel_id);
            // O servidor também precisa saber, ou a menção volta no próximo
            // dispositivo.
            let ids = ws.store.take_open_notifications(&channel_id);
            if !ids.is_empty() && !ws.store.me.is_empty() {
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
        // Na demonstração as ações mexem só no estado local.
        if self.demo {
            self.handle_chat_demo(ctx, action);
            return;
        }
        let s = self.settings.lang.strings();
        let ws = &mut self.workspaces[self.active];
        match action {
            ChatAction::Send {
                content,
                reply_to,
                attachments,
            } => {
                let channel_id = ws.store.selected_channel.clone();
                // Sem canal não há para onde mandar. Engolir a mensagem aqui
                // fazia o envio parecer quebrado: a caixa esvaziava e nada
                // acontecia, sem uma palavra de explicação.
                if channel_id.is_empty() {
                    ws.store.error = Some(s.no_channel_selected.to_owned());
                    return;
                }
                // Com anexo não há eco otimista: o servidor é quem sabe o que
                // saiu do upload.
                if attachments.is_empty() {
                    ws.store
                        .push_pending(&channel_id, &content, reply_to.clone());
                }
                ws.net.send(Command::SendMessage {
                    channel_id,
                    content,
                    reply_to,
                    attachments,
                });
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
                ws.store.toggle_reaction_local(&message_id, &emoji);
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
            ChatAction::NewChannel => {
                self.channel_dialog = Some(ChannelDialog::create())
            }
            ChatAction::EditChannel(id) => {
                if let Some(channel) = ws.store.channel(&id) {
                    self.channel_dialog = Some(ChannelDialog::edit(channel));
                }
            }
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
                ws.store.push_pending(&channel_id, &content, reply_to);
                if let Some(message) = ws.store.messages.last_mut() {
                    message.pending = false;
                }
            }
            ChatAction::Edit {
                message_id,
                content,
            } => {
                if let Some(message) = ws
                    .store
                    .messages
                    .iter_mut()
                    .find(|message| message.id == message_id)
                {
                    message.content = content;
                    message.edited = true;
                }
            }
            ChatAction::Delete(message_id) => {
                ws.store.messages.retain(|message| message.id != message_id)
            }
            ChatAction::React {
                message_id, emoji, ..
            } => {
                ws.store.toggle_reaction_local(&message_id, &emoji);
            }
            ChatAction::Pin { message_id, pin } => {
                if let Some(message) = ws
                    .store
                    .messages
                    .iter_mut()
                    .find(|message| message.id == message_id)
                {
                    message.pinned = pin;
                }
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
            // Na demonstração não há rede: o canal nasce, muda e some aqui
            // mesmo, que é o que deixa a tela de canais trabalhável sem
            // backend.
            ChatAction::NewChannel => {
                self.channel_dialog = Some(ChannelDialog::create())
            }
            ChatAction::EditChannel(id) => {
                if let Some(channel) = ws.store.channel(&id) {
                    self.channel_dialog = Some(ChannelDialog::edit(channel));
                }
            }
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
            // Sem rede na demonstração: estas quatro não têm efeito local.
            ChatAction::ChannelNotifications { .. }
            | ChatAction::MoveChannel { .. }
            | ChatAction::BanUser { .. }
            | ChatAction::ResetUser(_) => {}
            ChatAction::PickFiles => self.dialogs.pick_files(ctx.clone()),
            ChatAction::PickGif => self.dialogs.pick_animations(ctx.clone()),
            ChatAction::OpenExternally(path) => files::open_path(&path),
        }
    }

    /// Lê o que chegou de cada servidor. Todos são atendidos no mesmo
    /// quadro: um servidor que não está na tela ainda precisa contar as
    /// menções e disparar a notificação.
    fn pump_network(&mut self) {
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
                let ws = &mut self.workspaces[index];
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
                if let Some(server) = &ws.store.server {
                    if !server.name.is_empty() && ws.label != server.name {
                        ws.label = server.name.clone();
                        self.settings.servers[index].label = server.name.clone();
                    }
                }
            }
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
        let Some(action) = crate::ui::rail::draw(ui, &entries, self.active, &self.tokens, s) else {
            return;
        };
        match action {
            crate::ui::rail::RailAction::Select(index) => self.activate(index, ctx),
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
                } => {
                    let command = match purpose {
                        ImagePick::Avatar => Command::SetAvatar { blob, format },
                        ImagePick::Emoji(name) => Command::CreateEmoji { name, blob, format },
                    };
                    self.workspaces[self.active].net.send(command);
                }
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
        let screen = ctx.viewport_rect();
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
            MenuCommand::Preferences => self.settings_open = true,
            MenuCommand::SwitchLanguage(lang) => self.settings.lang = lang,
            MenuCommand::Quit => self.quit(ctx),
            // Marcar tudo como lido limpa todos os servidores: é o que o
            // contador da bandeja está somando.
            MenuCommand::MarkAllRead => {
                for ws in &mut self.workspaces {
                    ws.store.mark_all_read();
                }
            }
            MenuCommand::NewChannel => {
                self.channel_dialog = Some(ChannelDialog::create())
            }
            MenuCommand::Search => {
                self.search = Some(String::new());
                self.search_focus = true;
            }
            MenuCommand::About => self.about_open = true,
            MenuCommand::Roles => {
                self.roles.open = true;
                self.workspaces[self.active].net.send(Command::LoadRoles);
            }
            MenuCommand::Profile => self.admin.open_profile(),
            MenuCommand::ServerSettings => self.admin.open_server(),
        }
    }

    /// Janela de busca: termo em cima, resultados embaixo. Clicar num
    /// resultado abre o canal dele.
    fn search_window(&mut self, ctx: &egui::Context) {
        let Some(mut text) = self.search.clone() else {
            return;
        };
        let s = self.settings.lang.strings();
        let t = self.tokens;
        let mut open = true;
        let mut run = false;
        let mut jump: Option<String> = None;

        egui::Window::new(s.menu_search)
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(460.0)
            .default_height(420.0)
            .frame(
                egui::Frame::new()
                    .fill(t.elevated_bg)
                    .corner_radius(egui::CornerRadius::same(theme::radius::SHEET))
                    .inner_margin(egui::Margin::same(theme::space::XL as i8))
                    .stroke(egui::Stroke::new(1.0, t.separator))
                    .shadow(ctx.global_style().visuals.window_shadow),
            )
            .show(ctx, |ui| {
                let field = ui.add(
                    egui::TextEdit::singleline(&mut text)
                        .hint_text(s.search_placeholder)
                        .desired_width(f32::INFINITY)
                        .font(theme::text::body()),
                );
                if self.search_focus {
                    field.request_focus();
                    self.search_focus = false;
                }
                if field.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    run = true;
                }
                ui.add_space(theme::space::XS);
                ui.label(
                    egui::RichText::new(s.search_hint)
                        .font(theme::text::footnote())
                        .color(t.label_tertiary),
                );
                ui.add_space(theme::space::LG);

                let ws = &self.workspaces[self.active];
                if ws.store.searching {
                    ui.label(
                        egui::RichText::new(s.searching)
                            .font(theme::text::body())
                            .color(t.label_secondary),
                    );
                    return;
                }
                if ws.store.search_results.is_empty() {
                    ui.label(
                        egui::RichText::new(s.search_empty)
                            .font(theme::text::body())
                            .color(t.label_tertiary),
                    );
                    return;
                }

                egui::ScrollArea::vertical().show(ui, |ui| {
                    for found in &ws.store.search_results {
                        let when = found
                            .created_at
                            .map(|at| {
                                at.with_timezone(&chrono::Local)
                                    .format("%d/%m %H:%M")
                                    .to_string()
                            })
                            .unwrap_or_default();
                        let header = format!(
                            "#{} · {} · {when}",
                            found.channel_name, found.author_username
                        );
                        let row = ui
                            .scope(|ui| {
                                ui.label(
                                    egui::RichText::new(header)
                                        .font(theme::text::caption())
                                        .color(t.label_tertiary),
                                );
                                ui.label(
                                    egui::RichText::new(&found.content)
                                        .font(theme::text::body())
                                        .color(t.label),
                                );
                            })
                            .response
                            .interact(egui::Sense::click());
                        if row.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if row.clicked() && !found.channel_id.is_empty() {
                            jump = Some(found.channel_id.clone());
                        }
                        ui.add_space(theme::space::SM);
                        ui.separator();
                        ui.add_space(theme::space::SM);
                    }
                });
            });

        if run && !text.trim().is_empty() {
            let query = text.trim().to_owned();
            let ws = &mut self.workspaces[self.active];
            ws.store.searching = true;
            ws.net.send(Command::Search { text: query });
        }
        if let Some(channel_id) = jump {
            self.workspaces[self.active].store.selected_channel = channel_id;
            open = false;
        }

        self.search = open.then_some(text);
        if !open {
            let ws = &mut self.workspaces[self.active];
            ws.store.search_results.clear();
            ws.store.searching = false;
        }
    }

    /// Tela de cargos. Ela só descreve o que quer; a tradução em comandos
    /// de rede é aqui.
    fn roles_window(&mut self, ctx: &egui::Context) {
        use crate::ui::roles::RoleAction;

        let s = self.settings.lang.strings();
        let t = self.tokens;
        let actions = {
            let ws = &self.workspaces[self.active];
            crate::ui::roles::window(ctx, &mut self.roles, &ws.store, &t, s)
        };
        if actions.is_empty() {
            return;
        }
        let ws = &mut self.workspaces[self.active];
        for action in actions {
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
                RoleAction::Assign { user_id, role_id } => {
                    Command::AssignRole { user_id, role_id }
                }
                RoleAction::Unassign { user_id, role_id } => {
                    Command::UnassignRole { user_id, role_id }
                }
            };
            ws.net.send(command);
        }
    }

    /// Perfil e servidor. As duas telas só descrevem o que querem.
    fn admin_windows(&mut self, ctx: &egui::Context) {
        use crate::ui::admin::AdminAction;

        let s = self.settings.lang.strings();
        let t = self.tokens;
        let mut actions = {
            let ws = &self.workspaces[self.active];
            let mut found = crate::ui::admin::profile_window(ctx, &mut self.admin, &ws.store, &t, s);
            found.extend(crate::ui::admin::server_window(
                ctx,
                &mut self.admin,
                &ws.store,
                &t,
                s,
            ));
            found
        };
        if actions.is_empty() {
            return;
        }
        for action in actions.drain(..) {
            match action {
                // Os dois seletores de imagem passam pelo portal do sistema,
                // que responde noutro quadro.
                AdminAction::PickAvatar => self.dialogs.pick_image(ctx.clone(), ImagePick::Avatar),
                AdminAction::PickEmoji(name) => {
                    self.dialogs.pick_image(ctx.clone(), ImagePick::Emoji(name))
                }
                other => {
                    let command = match other {
                        AdminAction::SaveProfile(request) => Command::UpdateProfile(request),
                        AdminAction::SetPresence(status) => Command::SetStatus { status },
                        AdminAction::ChangePassword(password) => {
                            Command::ChangePassword { password }
                        }
                        AdminAction::LoadDevices => Command::LoadDevices,
                        AdminAction::DropConnection(connection_id) => {
                            Command::DropConnection { connection_id }
                        }
                        AdminAction::SaveServer(request) => Command::UpdateServer(request),
                        AdminAction::DeleteEmoji(emoji_id) => Command::DeleteEmoji { emoji_id },
                        AdminAction::LoadAuditLogs => Command::LoadAuditLogs,
                        AdminAction::PickAvatar | AdminAction::PickEmoji(_) => unreachable!(),
                    };
                    self.workspaces[self.active].net.send(command);
                }
            }
        }
    }

    /// Sobre: nome, versão e a que servidor a janela está ligada.
    fn about_window(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        let s = self.settings.lang.strings();
        let t = self.tokens;
        let mut open = self.about_open;
        let address = self.workspaces[self.active].url.clone();

        egui::Window::new(s.menu_about)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(320.0)
            .frame(
                egui::Frame::new()
                    .fill(t.elevated_bg)
                    .corner_radius(egui::CornerRadius::same(theme::radius::SHEET))
                    .inner_margin(egui::Margin::same(theme::space::XL as i8))
                    .stroke(egui::Stroke::new(1.0, t.separator))
                    .shadow(ctx.global_style().visuals.window_shadow),
            )
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Papo")
                        .font(theme::text::title1())
                        .color(t.label),
                );
                ui.add_space(theme::space::XS);
                ui.label(
                    egui::RichText::new(format!("v{}", env!("CARGO_PKG_VERSION")))
                        .font(theme::text::footnote())
                        .color(t.label_tertiary),
                );
                ui.add_space(theme::space::LG);
                ui.label(
                    egui::RichText::new(s.about_body)
                        .font(theme::text::body())
                        .color(t.label_secondary),
                );
                ui.add_space(theme::space::LG);
                ui.label(
                    egui::RichText::new(address)
                        .font(theme::text::footnote())
                        .color(t.label_tertiary),
                );
            });

        self.about_open = open;
    }

    /// O mesmo formulário, aplicado ao estado local da demonstração.
    fn submit_channel_demo(&mut self, dialog: &ChannelDialog) {
        use crate::state::{Channel, ChannelKind};

        let kind = match dialog.kind.as_str() {
            "voice" => ChannelKind::Voice,
            "category" => ChannelKind::Category,
            _ => ChannelKind::Text,
        };
        let topic = (dialog.kind != "category" && !dialog.topic.trim().is_empty())
            .then(|| dialog.topic.trim().to_owned());
        let name = dialog.name.trim().to_owned();
        let ws = &mut self.workspaces[self.active];

        match &dialog.id {
            Some(id) => {
                if let Some(channel) = ws
                    .store
                    .channels
                    .iter_mut()
                    .find(|channel| &channel.id == id)
                {
                    channel.name = name;
                    channel.topic = topic;
                }
            }
            None => {
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
        }
    }

    /// Diálogo de criar/editar canal. Mesma moldura dos ajustes.
    fn channel_window(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.channel_dialog.clone() else {
            return;
        };
        let s = self.settings.lang.strings();
        let t = self.tokens;
        let editing = dialog.id.is_some();
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;

        egui::Window::new(if editing {
            s.edit_channel_title
        } else {
            s.new_channel_title
        })
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(340.0)
        .frame(
            egui::Frame::new()
                .fill(t.elevated_bg)
                .corner_radius(egui::CornerRadius::same(theme::radius::SHEET))
                .inner_margin(egui::Margin::same(theme::space::XL as i8))
                .stroke(egui::Stroke::new(1.0, t.separator))
                .shadow(ctx.global_style().visuals.window_shadow),
        )
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(s.channel_name)
                    .font(theme::text::caption())
                    .color(t.label_tertiary),
            );
            ui.add_space(theme::space::XS);
            let name = ui.add(
                egui::TextEdit::singleline(&mut dialog.name)
                    .char_limit(32)
                    .desired_width(f32::INFINITY)
                    .font(theme::text::body()),
            );
            if dialog.focus {
                name.request_focus();
                dialog.focus = false;
            }

            // O tipo é escolhido uma vez. Só texto e categoria: apesar de o
            // openapi.yml listar `voice` no enum, o handler recusa — a
            // mensagem de erro dele diz "type deve ser 'text' ou 'category'".
            ui.add_space(theme::space::LG);
            ui.label(
                egui::RichText::new(s.channel_kind)
                    .font(theme::text::caption())
                    .color(t.label_tertiary),
            );
            ui.add_space(theme::space::XS);
            if editing {
                ui.label(
                    egui::RichText::new(match dialog.kind.as_str() {
                        "voice" => s.channel_kind_voice,
                        "category" => s.channel_kind_category,
                        _ => s.channel_kind_text,
                    })
                    .font(theme::text::body())
                    .color(t.label_secondary),
                );
            } else {
                ui.horizontal(|ui| {
                    for (value, label) in
                        [("text", s.channel_kind_text), ("category", s.channel_kind_category)]
                    {
                        let selected = dialog.kind == value;
                        if ui.add(egui::Button::selectable(selected, label)).clicked() {
                            dialog.kind = value.to_owned();
                        }
                    }
                });
            }

            // Categoria não tem tópico, e o contrato recusa um se vier.
            if dialog.kind != "category" {
                ui.add_space(theme::space::LG);
                ui.label(
                    egui::RichText::new(s.channel_topic)
                        .font(theme::text::caption())
                        .color(t.label_tertiary),
                );
                ui.add_space(theme::space::XS);
                ui.add(
                    egui::TextEdit::singleline(&mut dialog.topic)
                        .char_limit(512)
                        .desired_width(f32::INFINITY)
                        .font(theme::text::body()),
                );
            }

            ui.add_space(theme::space::XL);
            ui.horizontal(|ui| {
                let ready = !dialog.name.trim().is_empty();
                if ui
                    .add_enabled(
                        ready,
                        egui::Button::new(if editing { s.save } else { s.create_channel }),
                    )
                    .clicked()
                {
                    submit = true;
                }
                if ui.button(s.cancel).clicked() {
                    cancel = true;
                }
                // Enter no nome vale pelo botão.
                if ready
                    && name.lost_focus()
                    && ui.input(|input| input.key_pressed(egui::Key::Enter))
                {
                    submit = true;
                }
            });
        });

        if submit && self.demo {
            self.submit_channel_demo(&dialog);
        } else if submit {
            let name = dialog.name.trim().to_owned();
            // Categoria não aceita tópico; nos outros, vazio limpa o que
            // havia, e é por isso que ele vai mesmo em branco.
            let topic = (dialog.kind != "category").then(|| dialog.topic.trim().to_owned());
            let ws = &mut self.workspaces[self.active];
            match dialog.id.clone() {
                Some(channel_id) => ws.net.send(Command::UpdateChannel {
                    channel_id,
                    name,
                    topic,
                }),
                None => ws.net.send(Command::CreateChannel {
                    name,
                    kind: dialog.kind.clone(),
                    topic: topic.filter(|topic| !topic.is_empty()),
                }),
            }
        }

        if submit || cancel || !open {
            self.channel_dialog = None;
        } else {
            self.channel_dialog = Some(dialog);
        }
    }

    fn settings_window(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let s = self.settings.lang.strings();
        let t = self.tokens;
        let mut open = self.settings_open;
        let mut changed = false;
        let mut choose_folder = None;

        egui::Window::new(s.settings)
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_width(360.0)
            .frame(
                egui::Frame::new()
                    .fill(t.elevated_bg)
                    .corner_radius(egui::CornerRadius::same(theme::radius::SHEET))
                    .inner_margin(egui::Margin::same(theme::space::XL as i8))
                    .stroke(egui::Stroke::new(1.0, t.separator))
                    .shadow(ctx.global_style().visuals.window_shadow),
            )
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new(s.appearance)
                        .font(theme::text::caption())
                        .color(t.label_tertiary),
                );
                ui.add_space(theme::space::SM);

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(s.theme).font(theme::text::body()));
                    ui.add_space(theme::space::MD);
                    for (pref, label) in [
                        (ThemePref::System, s.theme_system),
                        (ThemePref::Light, s.theme_light),
                        (ThemePref::Dark, s.theme_dark),
                    ] {
                        if ui
                            .selectable_label(self.settings.theme == pref, label)
                            .clicked()
                        {
                            self.settings.theme = pref;
                            changed = true;
                        }
                    }
                });

                ui.add_space(theme::space::MD);
                if ui
                    .checkbox(&mut self.settings.translucency, s.translucency)
                    .changed()
                {
                    changed = true;
                }
                ui.label(
                    egui::RichText::new(s.translucency_hint)
                        .font(theme::text::footnote())
                        .color(t.label_tertiary),
                );

                ui.add_space(theme::space::XL);
                ui.label(
                    egui::RichText::new(s.menu_notifications)
                        .font(theme::text::caption())
                        .color(t.label_tertiary),
                );
                ui.add_space(theme::space::SM);
                ui.checkbox(&mut self.settings.notifications, s.menu_notifications);
                ui.label(
                    egui::RichText::new(s.notifications_hint)
                        .font(theme::text::footnote())
                        .color(t.label_tertiary),
                );
                ui.add_space(theme::space::MD);
                ui.checkbox(&mut self.settings.close_to_tray, s.menu_close_to_tray);
                ui.label(
                    egui::RichText::new(s.close_to_tray_hint)
                        .font(theme::text::footnote())
                        .color(t.label_tertiary),
                );
                ui.add_space(theme::space::MD);
                ui.checkbox(&mut self.settings.badge, s.badge);
                ui.label(
                    egui::RichText::new(s.badge_hint)
                        .font(theme::text::footnote())
                        .color(t.label_tertiary),
                );

                ui.add_space(theme::space::XL);
                ui.label(
                    egui::RichText::new(s.downloads)
                        .font(theme::text::caption())
                        .color(t.label_tertiary),
                );
                ui.add_space(theme::space::SM);
                let ask = self.settings.downloads == DownloadMode::Ask;
                ui.horizontal(|ui| {
                    if ui.selectable_label(!ask, s.download_folder).clicked() && ask {
                        self.settings.downloads = DownloadMode::Folder(files::downloads_dir());
                    }
                    if ui.selectable_label(ask, s.download_ask).clicked() {
                        self.settings.downloads = DownloadMode::Ask;
                    }
                });
                if let DownloadMode::Folder(dir) = self.settings.downloads.clone() {
                    ui.add_space(theme::space::XS);
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(crate::ui::attachments::elide(
                                &dir.display().to_string(),
                                34,
                            ))
                            .font(theme::text::footnote())
                            .color(t.label_secondary),
                        );
                        if ui.button(s.download_choose).clicked() {
                            choose_folder = Some(dir);
                        }
                    });
                }
                ui.label(
                    egui::RichText::new(s.downloads_hint)
                        .font(theme::text::footnote())
                        .color(t.label_tertiary),
                );

                ui.add_space(theme::space::XL);
                if ui
                    .checkbox(&mut self.settings.topic_reveal, s.topic_reveal)
                    .changed()
                {
                    self.ui.reveal_topic = self.settings.topic_reveal;
                }
                ui.label(
                    egui::RichText::new(s.topic_reveal_hint)
                        .font(theme::text::footnote())
                        .color(t.label_tertiary),
                );
                ui.add_space(theme::space::MD);
                if ui
                    .checkbox(&mut self.settings.record_button, s.record_button)
                    .changed()
                {
                    self.ui.show_record = self.settings.record_button;
                }
                ui.label(
                    egui::RichText::new(s.record_button_hint)
                        .font(theme::text::footnote())
                        .color(t.label_tertiary),
                );

                ui.add_space(theme::space::XL);
                ui.label(
                    egui::RichText::new(s.language)
                        .font(theme::text::caption())
                        .color(t.label_tertiary),
                );
                ui.add_space(theme::space::SM);
                ui.horizontal(|ui| {
                    for lang in Lang::ALL {
                        if ui
                            .selectable_label(self.settings.lang == lang, lang.endonym())
                            .clicked()
                        {
                            self.settings.lang = lang;
                        }
                    }
                });
            });

        self.settings_open = open;
        if let Some(start) = choose_folder {
            self.dialogs.pick_folder(ctx.clone(), start);
        }
        if changed {
            self.retheme(ctx);
        }
    }
}

impl eframe::App for PapoApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.attach_window(frame);
        #[cfg(target_os = "linux")]
        {
            self.sync_menu();
            self.sync_blur_regions(&ctx);
        }
        #[cfg(target_os = "linux")]
        self.handle_window_lifecycle(&ctx);

        self.pump_network();

        let strings = self.settings.lang.strings();
        self.draw_header(ui);
        self.draw_rail(ui, strings, &ctx);

        let active = self.active;
        match self.workspaces[active].store.screen {
            Screen::Starting => auth::starting(ui, &self.tokens, strings),
            Screen::Auth => {
                let ws = &mut self.workspaces[active];
                match auth::sign_in(ui, &mut ws.form, &ws.store, &self.tokens, strings) {
                    AuthAction::SignIn => self.authenticate(false, &ctx),
                    AuthAction::Register => self.authenticate(true, &ctx),
                    AuthAction::UnlockServer => self.unlock_server(),
                    _ => {}
                }
            }
            Screen::NeedsServer => {
                let ws = &mut self.workspaces[active];
                if auth::create_server(ui, &mut ws.form, &ws.store, &self.tokens, strings)
                    == AuthAction::CreateServer
                {
                    ws.store.busy = true;
                    ws.net.send(Command::CreateServer {
                        name: ws.form.server_name.trim().to_owned(),
                    });
                }
            }
            Screen::Chat => {
                shell::draw(
                    ui,
                    &mut self.workspaces[active].store,
                    &mut self.ui,
                    &self.tokens,
                    strings,
                );
                self.pump_chat();
                let actions = std::mem::take(&mut self.ui.actions);
                for action in actions {
                    self.handle_chat(&ctx, action);
                }
            }
        }
        self.settings_window(&ctx);
        self.channel_window(&ctx);
        self.search_window(&ctx);
        self.about_window(&ctx);
        self.roles_window(&ctx);
        self.admin_windows(&ctx);
        self.pump_files(&ctx);

        if self.own_chrome {
            crate::ui::headerbar::resize_handles(&ctx);
        }

        let pending = std::mem::take(&mut self.ui.pending);
        for command in pending {
            self.handle(&ctx, command);
        }
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let (Some(gl), Some(glass)) = (gl, &self.ui.glass) {
            if let Ok(glass) = glass.lock() {
                glass.destroy(gl);
            }
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        // As marcas de leitura vivem no estado de cada servidor; só o ajuste
        // persiste, e cada servidor guarda as suas sob a própria chave.
        for ws in &self.workspaces {
            self.settings
                .server_marks
                .insert(crate::state::server_key(&ws.url), ws.store.read_marks.clone());
        }
        self.settings.servers = self
            .workspaces
            .iter()
            .map(|ws| ServerEntry {
                url: ws.url.clone(),
                label: ws.label.clone(),
            })
            .collect();
        self.settings.active = self.active;
        eframe::set_value(storage, eframe::APP_KEY, &self.settings);
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
                MenuNode::item(s.menu_profile, MenuCommand::Profile),
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
