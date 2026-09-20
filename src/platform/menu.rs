//! Modelo de menu independente de plataforma.
//!
//! O app descreve o menu uma vez; a barra desenhada dentro da janela (egui) e
//! o menu global do sistema (DBusMenu, no Linux) consomem o mesmo modelo.

/// Comando disparado ao acionar um item de menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    NewChannel,
    Roles,
    Profile,
    ServerSettings,
    Search,
    MarkAllRead,
    ToggleMembers,
    ToggleTranslucency,
    ToggleNotifications,
    ToggleBadge,
    ToggleTopicReveal,
    ToggleRecordButton,
    ToggleCloseToTray,
    /// Fechar a janela: recolhe para a bandeja ou encerra, conforme o ajuste.
    /// Só a nossa barra de título manda isto; o menu do painel não o mostra.
    CloseWindow,
    SignOut,
    Preferences,
    SwitchLanguage(crate::i18n::Lang),
    About,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuKind {
    Normal,
    Separator,
    Checkbox { checked: bool },
    Radio { selected: bool },
    Submenu,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuNode {
    pub command: Option<MenuCommand>,
    pub label: String,
    pub kind: MenuKind,
    pub enabled: bool,
    /// Atalho no formato DBusMenu: `[["Control", "Q"]]`.
    pub accel: Option<Vec<String>>,
    pub children: Vec<MenuNode>,
}

impl MenuNode {
    pub fn item(label: impl Into<String>, command: MenuCommand) -> Self {
        Self {
            command: Some(command),
            label: label.into(),
            kind: MenuKind::Normal,
            enabled: true,
            accel: None,
            children: Vec::new(),
        }
    }

    pub fn submenu(label: impl Into<String>, children: Vec<MenuNode>) -> Self {
        Self {
            command: None,
            label: label.into(),
            kind: MenuKind::Submenu,
            enabled: true,
            accel: None,
            children,
        }
    }

    pub fn separator() -> Self {
        Self {
            command: None,
            label: String::new(),
            kind: MenuKind::Separator,
            enabled: true,
            accel: None,
            children: Vec::new(),
        }
    }

    pub fn checkbox(label: impl Into<String>, command: MenuCommand, checked: bool) -> Self {
        Self {
            kind: MenuKind::Checkbox { checked },
            ..Self::item(label, command)
        }
    }

    pub fn radio(label: impl Into<String>, command: MenuCommand, selected: bool) -> Self {
        Self {
            kind: MenuKind::Radio { selected },
            ..Self::item(label, command)
        }
    }

    pub fn accel(mut self, keys: &[&str]) -> Self {
        self.accel = Some(keys.iter().map(|k| (*k).to_string()).collect());
        self
    }

    #[allow(dead_code)] // itens desabilitados chegam com as permissões
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }
}

/// Árvore completa do menu: os nós de primeiro nível são os títulos da barra.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuModel {
    pub roots: Vec<MenuNode>,
}

impl MenuModel {
    pub fn new(roots: Vec<MenuNode>) -> Self {
        Self { roots }
    }
}
