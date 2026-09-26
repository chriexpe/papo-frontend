//! Ícone na bandeja do sistema (StatusNotifierItem).
//!
//! O KDE fala SNI nativamente; o ksni cuida do D-Bus numa thread própria.
//! A janela conversa com ele por canais, como no menu global.

use std::sync::mpsc;

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, Status, ToolTip};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    /// Clique no ícone: mostra ou esconde a janela.
    Toggle,
    Show,
    Quit,
}

/// Rótulos do menu da bandeja, traduzidos pela interface.
#[derive(Clone, Debug)]
pub struct TrayLabels {
    pub open: String,
    pub quit: String,
    pub tooltip: String,
}

struct PapoTray {
    icons: Vec<Icon>,
    labels: TrayLabels,
    /// Menções esperando; é o número que o painel mostra.
    mentions: u32,
    /// Alguma conversa com coisa nova, mesmo sem menção.
    unread: bool,
    commands: mpsc::Sender<TrayCommand>,
    repaint: egui::Context,
}

impl PapoTray {
    fn emit(&self, command: TrayCommand) {
        let _ = self.commands.send(command);
        self.repaint.request_repaint();
    }
}

impl ksni::Tray for PapoTray {
    fn id(&self) -> String {
        crate::APP_ID.into()
    }

    /// Alguns painéis mostram o título ao lado do ícone: o número vai junto.
    fn title(&self) -> String {
        if self.mentions > 0 {
            format!("Papo ({})", self.mentions)
        } else {
            "Papo".into()
        }
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icons.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Papo".into(),
            description: if self.mentions > 0 {
                format!("{} · {}", self.labels.tooltip, self.mentions)
            } else {
                self.labels.tooltip.clone()
            },
            ..Default::default()
        }
    }

    /// Com menção ou conversa nova, o painel destaca o ícone.
    fn status(&self) -> Status {
        if self.mentions > 0 || self.unread {
            Status::NeedsAttention
        } else {
            Status::Active
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.emit(TrayCommand::Toggle);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: self.labels.open.clone(),
                activate: Box::new(|tray: &mut Self| tray.emit(TrayCommand::Show)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: self.labels.quit.clone(),
                activate: Box::new(|tray: &mut Self| tray.emit(TrayCommand::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub struct Tray {
    handle: Handle<PapoTray>,
    commands: mpsc::Receiver<TrayCommand>,
}

impl Tray {
    /// `None` quando não há um host SNI na sessão.
    pub fn spawn(repaint: egui::Context, labels: TrayLabels) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let tray = PapoTray {
            icons: load_icons(),
            labels,
            mentions: 0,
            unread: false,
            commands: tx,
            repaint,
        };
        // O Flatpak não deixa registrar o nome próprio do item
        // (`StatusNotifierItem-<pid>-<n>`): o curinga do sandbox só alcança
        // componentes separados por ponto, e ali o que muda vem depois de um
        // hífen. Sem o nome, o item se registra pelo nome único da conexão —
        // que é o que o ksni recomenda justamente para sandbox.
        match tray
            .disable_dbus_name(crate::platform::in_flatpak())
            .spawn()
        {
            Ok(handle) => {
                log::info!("ícone de bandeja publicado");
                Some(Self {
                    handle,
                    commands: rx,
                })
            }
            Err(error) => {
                log::warn!("sem ícone de bandeja: {error}");
                None
            }
        }
    }

    pub fn try_recv(&self) -> Option<TrayCommand> {
        self.commands.try_recv().ok()
    }

    pub fn set_badge(&self, mentions: u32, unread: bool) {
        self.handle.update(|tray| {
            if tray.mentions != mentions || tray.unread != unread {
                tray.mentions = mentions;
                tray.unread = unread;
            }
        });
    }

    pub fn set_labels(&self, labels: TrayLabels) {
        self.handle.update(|tray| {
            if tray.labels.open != labels.open || tray.labels.quit != labels.quit {
                tray.labels = labels;
            }
        });
    }
}

/// O ícone do aplicativo em alguns tamanhos, em ARGB32.
fn load_icons() -> Vec<Icon> {
    let Ok(source) = image::load_from_memory(include_bytes!("../../../assets/icon.png")) else {
        return Vec::new();
    };
    [22u32, 32, 64]
        .into_iter()
        .map(|size| {
            let scaled = source
                .resize_exact(size, size, image::imageops::FilterType::Lanczos3)
                .to_rgba8();
            let data = scaled
                .pixels()
                .flat_map(|pixel| [pixel[3], pixel[0], pixel[1], pixel[2]])
                .collect();
            Icon {
                width: size as i32,
                height: size as i32,
                data,
            }
        })
        .collect()
}
