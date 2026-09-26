//! Native Windows notification-area icon.
//!
//! The eframe/winit event loop is already running on the UI thread when
//! PapoApp is constructed, which is exactly what tray-icon requires on
//! Windows. Events are forwarded into a tiny command queue and wake egui.

#![cfg(target_os = "windows")]

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

use tray_icon::{
    MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    Toggle,
    Show,
    Quit,
}

#[derive(Clone, Debug)]
pub struct TrayLabels {
    pub open: String,
    pub quit: String,
    pub tooltip: String,
}

pub struct Tray {
    icon: TrayIcon,
    open: MenuItem,
    quit: MenuItem,
    commands: mpsc::Receiver<TrayCommand>,
    quit_requested: Arc<AtomicBool>,
    tooltip: std::cell::RefCell<String>,
}

impl Tray {
    pub fn spawn(repaint: egui::Context, labels: TrayLabels) -> Option<Self> {
        let source = image::load_from_memory(include_bytes!("../../../assets/icon.png"))
            .ok()?
            .resize_exact(32, 32, image::imageops::FilterType::Lanczos3)
            .to_rgba8();
        let icon = tray_icon::Icon::from_rgba(source.into_raw(), 32, 32).ok()?;

        let menu = Menu::new();
        let open = MenuItem::new(&labels.open, true, None);
        let separator = PredefinedMenuItem::separator();
        let quit = MenuItem::new(&labels.quit, true, None);
        if menu
            .append_items(&[&open, &separator, &quit])
            .is_err()
        {
            return None;
        }

        let tray = match TrayIconBuilder::new()
            .with_id(crate::APP_ID)
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(&labels.tooltip)
            .with_icon(icon)
            .build()
        {
            Ok(tray) => tray,
            Err(error) => {
                log::warn!("sem ícone de bandeja no Windows: {error}");
                return None;
            }
        };

        let (tx, rx) = mpsc::channel();
        let quit_requested = Arc::new(AtomicBool::new(false));

        let open_id = open.id().clone();
        let quit_id = quit.id().clone();
        let menu_tx = tx.clone();
        let menu_repaint = repaint.clone();
        let menu_quit = Arc::clone(&quit_requested);
        MenuEvent::set_event_handler(Some(move |event| {
            if event.id == open_id {
                let _ = menu_tx.send(TrayCommand::Show);
                menu_repaint.request_repaint();
            } else if event.id == quit_id {
                menu_quit.store(true, Ordering::Release);
                // This bypasses close-to-tray even if Windows does not render
                // another frame while the window is minimized/hidden.
                menu_repaint.send_viewport_cmd(egui::ViewportCommand::Close);
                menu_repaint.request_repaint();
            }
        }));

        let tray_tx = tx;
        let tray_repaint = repaint;
        TrayIconEvent::set_event_handler(Some(move |event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                let _ = tray_tx.send(TrayCommand::Toggle);
                tray_repaint.request_repaint();
            }
        }));

        log::info!("ícone de bandeja do Windows publicado");
        Some(Self {
            icon: tray,
            open,
            quit,
            commands: rx,
            quit_requested,
            tooltip: std::cell::RefCell::new(labels.tooltip),
        })
    }

    pub fn try_recv(&self) -> Option<TrayCommand> {
        self.commands.try_recv().ok()
    }

    pub fn take_quit_requested(&self) -> bool {
        self.quit_requested.swap(false, Ordering::AcqRel)
    }

    pub fn set_badge(&self, mentions: u32, unread: bool) {
        let base = self.tooltip.borrow().clone();
        let tooltip = if mentions > 0 {
            format!("{base} · {mentions}")
        } else if unread {
            format!("{base} · nova mensagem")
        } else {
            base
        };
        let _ = self.icon.set_tooltip(Some(tooltip));
    }

    pub fn set_labels(&self, labels: TrayLabels) {
        if self.open.text() != labels.open {
            self.open.set_text(&labels.open);
        }
        if self.quit.text() != labels.quit {
            self.quit.set_text(&labels.quit);
        }
        if *self.tooltip.borrow() != labels.tooltip {
            *self.tooltip.borrow_mut() = labels.tooltip.clone();
            let _ = self.icon.set_tooltip(Some(labels.tooltip));
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        MenuEvent::set_event_handler(None);
        TrayIconEvent::set_event_handler(None);
        let _ = self.icon.set_visible(false);
    }
}
