//! Ícone na bandeja do sistema (StatusNotifierItem).
//!
//! O KDE fala SNI nativamente; o ksni cuida do D-Bus numa thread própria.
//! A janela conversa com ele por canais, como no menu global.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, Status, ToolTip};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    /// Clique no ícone: mostra ou esconde a janela.
    Toggle,
    Show,
    RestartRichPresence,
    Quit,
}

/// Rótulos do menu da bandeja, traduzidos pela interface.
#[derive(Clone, Debug)]
pub struct TrayLabels {
    pub open: String,
    pub restart_rich_presence: String,
    pub quit: String,
    pub tooltip: String,
}

struct PapoTray {
    icons: Vec<Icon>,
    badged_icons: Vec<Icon>,
    labels: TrayLabels,
    /// Menções esperando; é o número que o painel mostra.
    mentions: u32,
    /// Alguma conversa com coisa nova, mesmo sem menção.
    unread: bool,
    commands: mpsc::Sender<TrayCommand>,
    repaint: egui::Context,
    quit_requested: Arc<AtomicBool>,
}

impl PapoTray {
    fn emit(&self, command: TrayCommand) {
        let _ = self.commands.send(command);
        self.repaint.request_repaint();
    }

    fn request_quit(&self) {
        self.quit_requested.store(true, Ordering::Release);
        // Do not wait for a minimized Wayland window to render another frame
        // before asking the native event loop to close.
        self.repaint.send_viewport_cmd(egui::ViewportCommand::Close);
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
        if self.mentions > 0 || self.unread {
            self.badged_icons.clone()
        } else {
            self.icons.clone()
        }
    }

    fn attention_icon_pixmap(&self) -> Vec<Icon> {
        self.badged_icons.clone()
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

    /// Mantém o ícone estático mesmo com mensagens não lidas. No Plasma,
    /// NeedsAttention faz a bandeja pulsar; a bolinha vermelha já é desenhada
    /// por icon_pixmap e continua visível enquanto houver pendências.
    fn status(&self) -> Status {
        Status::Active
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
                label: self.labels.restart_rich_presence.clone(),
                activate: Box::new(|tray: &mut Self| tray.emit(TrayCommand::RestartRichPresence)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: self.labels.quit.clone(),
                activate: Box::new(|tray: &mut Self| tray.request_quit()),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub struct Tray {
    handle: Handle<PapoTray>,
    commands: mpsc::Receiver<TrayCommand>,
    quit_requested: Arc<AtomicBool>,
    last_badge: std::cell::Cell<Option<(u32, bool)>>,
}

impl Tray {
    /// `None` quando não há um host SNI na sessão.
    pub fn spawn(repaint: egui::Context, labels: TrayLabels) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let quit_requested = Arc::new(AtomicBool::new(false));
        let tray = PapoTray {
            icons: load_icons(false),
            badged_icons: load_icons(true),
            labels,
            mentions: 0,
            unread: false,
            commands: tx,
            repaint,
            quit_requested: Arc::clone(&quit_requested),
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
                    quit_requested,
                    last_badge: std::cell::Cell::new(None),
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

    pub fn take_quit_requested(&self) -> bool {
        self.quit_requested.swap(false, Ordering::AcqRel)
    }

    pub fn set_badge(&self, mentions: u32, unread: bool) {
        if self.last_badge.replace(Some((mentions, unread))) == Some((mentions, unread)) {
            return;
        }
        self.handle.update(|tray| {
            if tray.mentions != mentions || tray.unread != unread {
                tray.mentions = mentions;
                tray.unread = unread;
            }
        });
    }

    pub fn set_labels(&self, labels: TrayLabels) {
        self.handle.update(|tray| {
            if tray.labels.open != labels.open
                || tray.labels.restart_rich_presence != labels.restart_rich_presence
                || tray.labels.quit != labels.quit
                || tray.labels.tooltip != labels.tooltip
            {
                tray.labels = labels;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unread_badge_does_not_request_panel_animation() {
        let (commands, _) = mpsc::channel();
        let quit_requested = Arc::new(AtomicBool::new(false));
        let mut tray = PapoTray {
            icons: vec![Icon { width: 1, height: 1, data: vec![255, 1, 1, 1] }],
            badged_icons: vec![Icon { width: 1, height: 1, data: vec![255, 229, 57, 53] }],
            labels: TrayLabels {
                open: String::new(),
                restart_rich_presence: String::new(),
                quit: String::new(),
                tooltip: String::new(),
            },
            mentions: 0,
            unread: false,
            commands,
            repaint: egui::Context::default(),
            quit_requested,
        };
        assert!(matches!(ksni::Tray::status(&tray), Status::Active));
        assert_eq!(ksni::Tray::icon_pixmap(&tray)[0].data, vec![255, 1, 1, 1]);

        tray.unread = true;
        assert!(matches!(ksni::Tray::status(&tray), Status::Active));
        assert_eq!(ksni::Tray::icon_pixmap(&tray)[0].data, vec![255, 229, 57, 53]);

        tray.mentions = 2;
        assert!(matches!(ksni::Tray::status(&tray), Status::Active));
        assert_eq!(ksni::Tray::icon_pixmap(&tray)[0].data, vec![255, 229, 57, 53]);
    }
}

/// O ícone do aplicativo em alguns tamanhos, em ARGB32.
fn load_icons(pending: bool) -> Vec<Icon> {
    [22u32, 32, 64]
        .into_iter()
        .filter_map(|size| {
            let scaled = crate::platform::tray_icon::render(size, pending)?;
            let data = scaled
                .pixels()
                .flat_map(|pixel| [pixel[3], pixel[0], pixel[1], pixel[2]])
                .collect();
            Some(Icon {
                width: size as i32,
                height: size as i32,
                data,
            })
        })
        .collect()
}
