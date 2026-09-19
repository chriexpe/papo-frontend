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
    unread: u32,
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
        "papo".into()
    }

    fn title(&self) -> String {
        "Papo".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        self.icons.clone()
    }

    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            title: "Papo".into(),
            description: if self.unread > 0 {
                format!("{} · {}", self.labels.tooltip, self.unread)
            } else {
                self.labels.tooltip.clone()
            },
            ..Default::default()
        }
    }

    /// Com mensagens por ler o painel destaca o ícone.
    fn status(&self) -> Status {
        if self.unread > 0 {
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
            unread: 0,
            commands: tx,
            repaint,
        };
        match tray.spawn() {
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

    pub fn set_unread(&self, unread: u32) {
        self.handle.update(|tray| {
            if tray.unread != unread {
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
    let Ok(source) = image::load_from_memory(include_bytes!("../../assets/icon.png")) else {
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
