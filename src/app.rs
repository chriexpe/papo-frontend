//! Aplicação: junta estado, tema e telas.

use egui::Color32;

use crate::i18n::Lang;
use crate::media::Media;
use crate::platform::files::{self, Chosen, Dialogs};
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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// Endereço do backend; trocar reabre a conexão.
    #[serde(default = "default_server_url")]
    pub server_url: String,
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
    /// Até quando cada canal foi visto; é o que sobrevive ao fechamento.
    #[serde(default)]
    pub read_marks: std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
}

fn enabled() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            server_url: default_server_url(),
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

pub struct PapoApp {
    store: Store,
    ui: UiState,
    settings: Settings,
    system: SystemTheme,
    tokens: Tokens,
    settings_open: bool,
    net: Net,
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
    form: AuthForm,
    /// Instante do último evento de digitação enviado.
    typing_sent: Option<std::time::Instant>,
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
}

impl PapoApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, eframe::APP_KEY))
            .unwrap_or_default();

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

        let net = Net::spawn(settings.server_url.clone(), cc.egui_ctx.clone());
        // A mídia usa o mesmo cookie da sessão para baixar anexos.
        let media = Media::spawn(
            settings.server_url.clone(),
            std::sync::Arc::clone(&net.session),
            cc.egui_ctx.clone(),
        );
        let demo = std::env::var("PAPO_DEMO").is_ok();
        let mut store = Store::default();
        store.read_marks = settings.read_marks.clone();
        if demo {
            crate::state::demo::seed(&mut store);
        }

        Self {
            store,
            ui: UiState {
                show_members: settings.show_members,
                translucent: settings.translucency,
                reveal_topic: settings.topic_reveal,
                show_record: settings.record_button,
                media: crate::media::MediaStore::new(media),
                glass,
                ..UiState::default()
            },
            net,
            #[cfg(target_os = "linux")]
            tray: Tray::spawn(cc.egui_ctx.clone(), tray_labels(&settings)),
            #[cfg(target_os = "linux")]
            notifier: Notifier::spawn(),
            #[cfg(target_os = "linux")]
            launcher: Launcher::spawn(),
            dialogs: Dialogs::default(),
            focused: true,
            quitting: false,
            form: AuthForm {
                server_url: settings.server_url.clone(),
                ..AuthForm::default()
            },
            typing_sent: None,
            settings,
            system,
            tokens,
            settings_open: false,
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
        }
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
            crate::platform::kwin::restore_window("papo");
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
        let mentions = if self.settings.badge {
            self.store.mention_total()
        } else {
            0
        };
        let unread = self.settings.badge && self.store.has_unread();

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
    fn maybe_notify(&self, update: &crate::api::net::Update) {
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
        if message.author_id == self.store.me {
            return;
        }

        let author = self
            .store
            .member(&message.author_id)
            .map(|member| member.name.clone())
            .unwrap_or_else(|| message.author_id.clone());
        let channel = self
            .store
            .channel(&message.channel_id)
            .map(|channel| format!("#{}", channel.name))
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
        let username = self.form.username.trim().to_owned();
        let password = self.form.password.clone();
        if username.is_empty() || password.is_empty() {
            return;
        }

        // Trocar de servidor exige uma conexão nova.
        let url = self.form.server_url.trim().to_owned();
        if !url.is_empty() && url != self.settings.server_url {
            self.settings.server_url = url.clone();
            self.net = Net::spawn(url, ctx.clone());
        }

        self.store.busy = true;
        self.store.error = None;
        self.net.send(if register {
            Command::Register { username, password }
        } else {
            Command::Login { username, password }
        });
    }

    /// Ações da conversa: mensagens novas, digitação e carga sob demanda.
    fn pump_chat(&mut self) {
        if self.demo {
            return;
        }
        if let Some(channel_id) = self.store.channel_needing_messages() {
            self.store.mark_loading(&channel_id);
            self.net.send(Command::LoadMessages {
                channel_id: channel_id.clone(),
            });
            self.net.send(Command::LoadPinned { channel_id });
        }

        // Com a janela à frente, o canal aberto está sendo lido agora.
        if self.focused && !self.minimized && !self.store.selected_channel.is_empty() {
            let channel_id = self.store.selected_channel.clone();
            self.store.mark_read(&channel_id);
            // O servidor também precisa saber, ou a menção volta no próximo
            // dispositivo.
            let ids = self.store.take_open_notifications(&channel_id);
            if !ids.is_empty() && !self.store.me.is_empty() {
                self.net.send(Command::MarkNotificationsRead {
                    user_id: self.store.me.clone(),
                    ids,
                });
            }
        }

        // Um evento de digitação a cada três segundos basta para o servidor.
        if self.ui.typed {
            let now = std::time::Instant::now();
            let stale = self
                .typing_sent
                .is_none_or(|last| now.duration_since(last).as_secs() >= 3);
            if stale && !self.store.selected_channel.is_empty() {
                self.typing_sent = Some(now);
                self.net.send(Command::Typing {
                    channel_id: self.store.selected_channel.clone(),
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
        match action {
            ChatAction::Send {
                content,
                reply_to,
                attachments,
            } => {
                let channel_id = self.store.selected_channel.clone();
                if channel_id.is_empty() {
                    return;
                }
                // Com anexo não há eco otimista: o servidor é quem sabe o que
                // saiu do upload.
                if attachments.is_empty() {
                    self.store
                        .push_pending(&channel_id, &content, reply_to.clone());
                }
                self.net.send(Command::SendMessage {
                    channel_id,
                    content,
                    reply_to,
                    attachments,
                });
            }
            ChatAction::Edit {
                message_id,
                content,
            } => self.net.send(Command::EditMessage {
                message_id,
                content,
            }),
            ChatAction::Delete(message_id) => {
                self.net.send(Command::DeleteMessage { message_id })
            }
            ChatAction::React {
                message_id,
                emoji,
                add,
            } => {
                let channel_id = self
                    .store
                    .message(&message_id)
                    .map(|message| message.channel_id.clone())
                    .unwrap_or_else(|| self.store.selected_channel.clone());
                // A reação aparece na hora; o contador certo vem pelo evento.
                self.store.toggle_reaction_local(&message_id, &emoji);
                self.net.send(Command::React {
                    channel_id,
                    message_id,
                    emoji: emoji.request(),
                    add,
                });
            }
            ChatAction::Pin { message_id, pin } => {
                let channel_id = self
                    .store
                    .message(&message_id)
                    .map(|message| message.channel_id.clone())
                    .unwrap_or_else(|| self.store.selected_channel.clone());
                self.net.send(Command::Pin {
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
            ChatAction::PickFiles => self.dialogs.pick_files(ctx.clone()),
            ChatAction::PickGif => self.dialogs.pick_animations(ctx.clone()),
            ChatAction::OpenExternally(path) => files::open_path(&path),
        }
    }

    /// A demonstração responde sozinha: sem rede, o estado é a verdade.
    fn handle_chat_demo(&mut self, ctx: &egui::Context, action: ChatAction) {
        match action {
            ChatAction::Send { content, reply_to, .. } => {
                let channel_id = self.store.selected_channel.clone();
                self.store.push_pending(&channel_id, &content, reply_to);
                if let Some(message) = self.store.messages.last_mut() {
                    message.pending = false;
                }
            }
            ChatAction::Edit {
                message_id,
                content,
            } => {
                if let Some(message) = self
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
                self.store.messages.retain(|message| message.id != message_id)
            }
            ChatAction::React {
                message_id, emoji, ..
            } => {
                self.store.toggle_reaction_local(&message_id, &emoji);
            }
            ChatAction::Pin { message_id, pin } => {
                if let Some(message) = self
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
            ChatAction::PickFiles => self.dialogs.pick_files(ctx.clone()),
            ChatAction::PickGif => self.dialogs.pick_animations(ctx.clone()),
            ChatAction::OpenExternally(path) => files::open_path(&path),
        }
    }

    /// Respostas dos diálogos do sistema e arquivos soltos na janela.
    fn pump_files(&mut self, ctx: &egui::Context) {
        for chosen in self.dialogs.poll() {
            match chosen {
                Chosen::Files(uploads) => self.ui.attachments.extend(uploads),
                Chosen::Folder(path) => self.settings.downloads = DownloadMode::Folder(path),
                Chosen::SaveAs { id, name, dest } => self.ui.media.save(&id, &name, dest),
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
        let mut regions = vec![(0, 0, crate::ui::shell::SIDEBAR_WIDTH as i32, screen.height() as i32)];
        if self.settings.show_members {
            let width = crate::ui::shell::MEMBERS_WIDTH as i32;
            regions.push((screen.width() as i32 - width, 0, width, screen.height() as i32));
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
            MenuCommand::SignOut => self.net.send(Command::Logout),
            MenuCommand::Preferences => self.settings_open = true,
            MenuCommand::SwitchLanguage(lang) => self.settings.lang = lang,
            MenuCommand::Quit => self.quit(ctx),
            MenuCommand::MarkAllRead => self.store.mark_all_read(),
            MenuCommand::NewChannel | MenuCommand::Search | MenuCommand::About => {}
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

        while let Some(update) = self.net.try_recv() {
            // No modo demonstração a rede não manda no estado.
            if self.demo {
                continue;
            }
            #[cfg(target_os = "linux")]
            self.maybe_notify(&update);
            // Reconectou: o que aconteceu durante a queda vem da carga nova.
            if matches!(
                update,
                crate::api::net::Update::Connection(crate::api::ws::Connection::Online)
            ) && self.store.screen == Screen::Chat
            {
                self.net.send(Command::Refresh);
            }
            self.store.apply(update);
        }

        let strings = self.settings.lang.strings();
        match self.store.screen {
            Screen::Starting => auth::starting(ui, &self.tokens, strings),
            Screen::Auth => {
                match auth::sign_in(ui, &mut self.form, &self.store, &self.tokens, strings) {
                    AuthAction::SignIn => self.authenticate(false, &ctx),
                    AuthAction::Register => self.authenticate(true, &ctx),
                    _ => {}
                }
            }
            Screen::NeedsServer => {
                if auth::create_server(ui, &mut self.form, &self.store, &self.tokens, strings)
                    == AuthAction::CreateServer
                {
                    self.store.busy = true;
                    self.net.send(Command::CreateServer {
                        name: self.form.server_name.trim().to_owned(),
                    });
                }
            }
            Screen::Chat => {
                shell::draw(ui, &mut self.store, &mut self.ui, &self.tokens, strings);
                self.pump_chat();
                let actions: Vec<_> = self.ui.actions.drain(..).collect();
                for action in actions {
                    self.handle_chat(&ctx, action);
                }
            }
        }
        self.settings_window(&ctx);
        self.pump_files(&ctx);

        let pending: Vec<_> = self.ui.pending.drain(..).collect();
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
        // As marcas de leitura vivem no estado, mas só o ajuste persiste.
        self.settings.read_marks = self.store.read_marks.clone();
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
