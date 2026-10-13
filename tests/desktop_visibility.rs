//! Requires a desktop session. Run with:
//! cargo test --test desktop_visibility -- --ignored --nocapture
//! Set PAPO_TEST_BACKEND=x11 to verify the X11 backend separately.
#![cfg(target_os = "linux")]

use eframe::{App, egui};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::time::{Duration, Instant};

struct Probe {
    start: Instant,
    phase: usize,
    logic_ticks: usize,
    ui_ticks: usize,
    hidden_ticks: usize,
}

impl App for Probe {
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.logic_ticks += 1;
        let mut map_state = None;
        let window = frame.winit_window().unwrap();
        // Verify the actual X server map state independently of cached visibility.
        let visible = match window.window_handle().unwrap().as_raw() {
            RawWindowHandle::Xlib(handle) => {
                use x11rb::protocol::xproto::ConnectionExt;
                let (connection, _) = x11rb::connect(None).unwrap();
                let attributes = connection
                    .get_window_attributes(handle.window as u32)
                    .unwrap()
                    .reply()
                    .unwrap();
                map_state = Some(attributes.map_state);
                Some(attributes.map_state == x11rb::protocol::xproto::MapState::VIEWABLE)
            }
            _ => window.is_visible(),
        };
        if visible == Some(false) {
            self.hidden_ticks += 1;
        }
        let deadlines = [1000, 3000, 5000, 7000, 9000, 11000, 13000];
        if self.phase < deadlines.len() && self.start.elapsed().as_millis() >= deadlines[self.phase]
        {
            if self.phase == 6 {
                assert!(self.hidden_ticks >= 6, "background logic stopped");
                assert!(
                    self.ui_ticks < self.logic_ticks,
                    "hidden UI was still painted"
                );
                assert_eq!(visible, Some(true), "last remap did not complete");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else {
                assert_eq!(
                    visible,
                    Some(self.phase % 2 == 0),
                    "phase {}, logic={}, ui={}, x11_map={:?}, native={:?}, window={:?}",
                    self.phase,
                    self.logic_ticks,
                    self.ui_ticks,
                    map_state,
                    window.is_visible(),
                    window.id()
                );
                if self.phase % 2 == 1 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(self.phase % 2 == 1));
                if self.phase % 2 == 1 {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
            }
            self.phase += 1;
        }
        ctx.request_repaint_after(Duration::from_millis(80));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        self.ui_ticks += 1;
        ui.label("Papo: teste de ocultação e restauração da janela");
    }

    fn on_exit(&mut self) {
        assert_eq!(self.phase, 7, "probe exited before completing its cycles");
    }
}

#[test]
#[ignore = "requires a desktop session and opens a temporary probe window"]
fn hidden_window_keeps_logic_running_and_can_be_remapped_repeatedly() {
    let _ = env_logger::try_init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_app_id("io.github.chriexpe.Papo.VisibilityProbe")
            .with_inner_size([320.0, 160.0]),
        event_loop_builder: Some(Box::new(|builder| {
            winit::platform::wayland::EventLoopBuilderExtWayland::with_any_thread(builder, true);
            winit::platform::x11::EventLoopBuilderExtX11::with_any_thread(builder, true);
            if std::env::var("PAPO_TEST_BACKEND").as_deref() == Ok("x11") {
                winit::platform::x11::EventLoopBuilderExtX11::with_x11(builder);
            }
        })),
        persist_window: false,
        ..Default::default()
    };
    eframe::run_native(
        "Papo visibility probe",
        options,
        Box::new(|_| {
            Ok(Box::new(Probe {
                start: Instant::now(),
                phase: 0,
                logic_ticks: 0,
                ui_ticks: 0,
                hidden_ticks: 0,
            }))
        }),
    )
    .unwrap();
}
