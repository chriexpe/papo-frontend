//! Textos da interface em pt-BR e inglês.
//!
//! Cada idioma é uma instância de [`Strings`]: o compilador exige que todo
//! campo novo seja preenchido nos dois, então nenhuma string fica sem tradução.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Lang {
    #[default]
    PtBr,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::PtBr, Lang::En];

    pub fn strings(self) -> &'static Strings {
        match self {
            Lang::PtBr => &PT_BR,
            Lang::En => &EN,
        }
    }

    /// Nome do idioma no próprio idioma.
    pub fn endonym(self) -> &'static str {
        match self {
            Lang::PtBr => "Português (Brasil)",
            Lang::En => "English",
        }
    }
}

pub struct Strings {
    // Menu
    pub menu_papo: &'static str,
    pub menu_file: &'static str,
    pub menu_view: &'static str,
    pub menu_help: &'static str,
    pub menu_new_channel: &'static str,
    pub menu_search: &'static str,
    pub menu_mark_all_read: &'static str,
    pub menu_toggle_members: &'static str,
    pub menu_notifications: &'static str,
    pub menu_close_to_tray: &'static str,
    pub menu_preferences: &'static str,
    pub menu_language: &'static str,
    pub menu_about: &'static str,
    pub menu_quit: &'static str,
    pub sign_out: &'static str,

    // Estrutura
    pub text_channels: &'static str,
    pub voice_channels: &'static str,
    pub members: &'static str,
    pub online: &'static str,
    pub away: &'static str,
    pub busy: &'static str,
    pub offline: &'static str,

    // Conversa
    pub composer_hint: &'static str,
    pub pinned: &'static str,
    pub search: &'static str,
    pub attach: &'static str,
    #[allow(dead_code)] // usado pela ação de responder
    pub reply: &'static str,
    pub edited: &'static str,
    pub today: &'static str,
    pub yesterday: &'static str,
    pub typing_one: &'static str,
    pub typing_many: &'static str,
    pub empty_channel_title: &'static str,
    pub empty_channel_body: &'static str,

    // Ajustes
    pub settings: &'static str,
    pub appearance: &'static str,
    pub translucency: &'static str,
    pub translucency_hint: &'static str,
    pub theme: &'static str,
    pub theme_system: &'static str,
    pub theme_light: &'static str,
    pub theme_dark: &'static str,
    pub language: &'static str,
    pub notifications_hint: &'static str,
    pub close_to_tray_hint: &'static str,
    pub tray_open: &'static str,
    pub tray_quit: &'static str,
    pub tray_tooltip: &'static str,

    // Sessão
    pub sign_in: &'static str,
    pub sign_up: &'static str,
    pub username: &'static str,
    pub password: &'static str,
    pub server: &'static str,
    pub connecting: &'static str,
    pub reconnecting: &'static str,
    pub offline_banner: &'static str,
}

pub static PT_BR: Strings = Strings {
    menu_papo: "Papo",
    menu_file: "Arquivo",
    menu_view: "Exibir",
    menu_help: "Ajuda",
    menu_new_channel: "Novo canal…",
    menu_search: "Buscar…",
    menu_mark_all_read: "Marcar tudo como lido",
    menu_toggle_members: "Lista de membros",
    menu_notifications: "Notificações",
    menu_close_to_tray: "Continuar em segundo plano",
    menu_preferences: "Ajustes…",
    menu_language: "Idioma",
    menu_about: "Sobre o Papo",
    menu_quit: "Sair",
    sign_out: "Sair da conta",

    text_channels: "Canais de texto",
    voice_channels: "Canais de voz",
    members: "Membros",
    online: "Online",
    away: "Ausente",
    busy: "Ocupado",
    offline: "Offline",

    composer_hint: "Mensagem em",
    pinned: "Fixadas",
    search: "Buscar",
    attach: "Anexar",
    reply: "Responder",
    edited: "editada",
    today: "Hoje",
    yesterday: "Ontem",
    typing_one: "está digitando…",
    typing_many: "estão digitando…",
    empty_channel_title: "Começo do canal",
    empty_channel_body: "Nenhuma mensagem por aqui ainda. Diga alguma coisa.",

    settings: "Ajustes",
    appearance: "Aparência",
    translucency: "Translucidez",
    translucency_hint: "Desliga o vidro fosco das barras e usa fundos opacos.",
    theme: "Tema",
    theme_system: "Sistema",
    theme_light: "Claro",
    theme_dark: "Escuro",
    language: "Idioma",
    notifications_hint: "Avisa quando chega mensagem e a janela não está em foco.",
    close_to_tray_hint: "Fechar recolhe a janela e o Papo segue rodando na bandeja.",
    tray_open: "Abrir o Papo",
    tray_quit: "Sair",
    tray_tooltip: "Conversas",

    sign_in: "Entrar",
    sign_up: "Criar conta",
    username: "Usuário",
    password: "Senha",
    server: "Servidor",
    connecting: "Conectando…",
    reconnecting: "Reconectando…",
    offline_banner: "Sem conexão com o servidor",
};

pub static EN: Strings = Strings {
    menu_papo: "Papo",
    menu_file: "File",
    menu_view: "View",
    menu_help: "Help",
    menu_new_channel: "New Channel…",
    menu_search: "Search…",
    menu_mark_all_read: "Mark All as Read",
    menu_toggle_members: "Member List",
    menu_notifications: "Notifications",
    menu_close_to_tray: "Keep Running in Background",
    menu_preferences: "Settings…",
    menu_language: "Language",
    menu_about: "About Papo",
    menu_quit: "Quit",
    sign_out: "Sign out",

    text_channels: "Text channels",
    voice_channels: "Voice channels",
    members: "Members",
    online: "Online",
    away: "Away",
    busy: "Busy",
    offline: "Offline",

    composer_hint: "Message",
    pinned: "Pinned",
    search: "Search",
    attach: "Attach",
    reply: "Reply",
    edited: "edited",
    today: "Today",
    yesterday: "Yesterday",
    typing_one: "is typing…",
    typing_many: "are typing…",
    empty_channel_title: "Start of the channel",
    empty_channel_body: "No messages here yet. Say something.",

    settings: "Settings",
    appearance: "Appearance",
    translucency: "Translucency",
    translucency_hint: "Turns off frosted bars and uses opaque backgrounds.",
    theme: "Theme",
    theme_system: "System",
    theme_light: "Light",
    theme_dark: "Dark",
    language: "Language",
    notifications_hint: "Alerts you when a message arrives and the window isn't focused.",
    close_to_tray_hint: "Closing minimises the window and Papo keeps running in the tray.",
    tray_open: "Open Papo",
    tray_quit: "Quit",
    tray_tooltip: "Conversations",

    sign_in: "Sign in",
    sign_up: "Create account",
    username: "Username",
    password: "Password",
    server: "Server",
    connecting: "Connecting…",
    reconnecting: "Reconnecting…",
    offline_banner: "No connection to the server",
};
