//! Aplicação: junta estado, tema e telas.

use egui::Color32;

use crate::i18n::Lang;
use crate::platform::menu::{MenuCommand, MenuModel, MenuNode};
use crate::platform::desktop;
#[cfg(target_os = "linux")]
use crate::platform::activate::Activator;
use crate::platform::notify::{Notification, Notifier};
use crate::platform::tray::{Tray, TrayCommand, TrayLabels};
use crate::platform::{appmenu::AppMenuSurface, blur::BlurSurface, global_menu::GlobalMenu};
use crate::api::net::{Command, Net};
use crate::state::{Screen, Store};
use crate::ui::auth::{self, AuthAction, AuthForm};
use crate::ui::shell::{self, UiState};
use crate::ui::glass::GlassRenderer;
use crate::ui::theme::{self, Appearance, ThemePref, Tokens};

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

        Self {
            store: Store::default(),
            ui: UiState {
                show_members: settings.show_members,
                translucent: settings.translucency,
                glass,
                ..UiState::default()
            },
            net: Net::spawn(settings.server_url.clone(), cc.egui_ctx.clone()),
            #[cfg(target_os = "linux")]
            tray: Tray::spawn(cc.egui_ctx.clone(), tray_labels(&settings)),
            #[cfg(target_os = "linux")]
            notifier: Notifier::spawn(),
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

        if let Some(tray) = &self.tray {
            let unread = self
                .store
                .channels
                .iter()
                .filter(|channel| channel.unread)
                .count() as u32;
            tray.set_unread(unread);
            tray.set_labels(tray_labels(&self.settings));
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
        if let Some(channel_id) = self.store.channel_needing_messages() {
            self.store.mark_loading(&channel_id);
            self.net.send(Command::LoadMessages { channel_id });
        }

        if let Some(content) = self.ui.outgoing.take() {
            let channel_id = self.store.selected_channel.clone();
            if !channel_id.is_empty() {
                self.store.push_pending(&channel_id, &content);
                self.net.send(Command::SendMessage {
                    channel_id,
                    content,
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
            MenuCommand::ToggleCloseToTray => {
                self.settings.close_to_tray = !self.settings.close_to_tray;
            }
            MenuCommand::SignOut => self.net.send(Command::Logout),
            MenuCommand::Preferences => self.settings_open = true,
            MenuCommand::SwitchLanguage(lang) => self.settings.lang = lang,
            MenuCommand::Quit => self.quit(ctx),
            MenuCommand::NewChannel | MenuCommand::Search | MenuCommand::MarkAllRead
            | MenuCommand::About => {}
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
            }
        }
        self.settings_window(&ctx);

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
