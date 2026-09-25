//! Linux WebEmbed backend: Servo rendered offscreen (software) into an egui
//! texture. Wayland and X11 use the same path; Papo never hands its eframe
//! context to Servo and never links GTK/Wry.
//!
//! Policy, identity, geometry and lifecycle stay in `crate::webembed`. This
//! module only knows how to drive a single Servo `WebView` and hand its pixels
//! back as a texture.

#![cfg(target_os = "linux")]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use egui::{ColorImage, TextureHandle, TextureOptions};
use euclid::Scale;
use servo::{
    DeviceIndependentPixel, DeviceIntPoint, DeviceIntRect, DeviceIntSize, DevicePixel, DevicePoint,
    EventLoopWaker, InputEvent, MouseButton, MouseButtonAction, MouseButtonEvent, MouseLeftViewportEvent,
    MouseMoveEvent, RenderingContext, Servo, ServoBuilder, SoftwareRenderingContext, WebView,
    WebViewBuilder, WebViewDelegate, WebViewPoint, WheelDelta, WheelEvent, WheelMode,
};

use crate::webembed::{EmbedViewport, WebEmbedBackend, WebEmbedButton, WebEmbedEvent, WebEmbedInput};

/// The egui context is needed for two things: waking the UI from Servo's event
/// loop, and uploading decoded frames as textures. The shell hands it over
/// every frame; it is process-global because Servo's types are not `Send`.
static CONTEXT: OnceLock<Mutex<Option<egui::Context>>> = OnceLock::new();

fn context_slot() -> &'static Mutex<Option<egui::Context>> {
    CONTEXT.get_or_init(|| Mutex::new(None))
}

/// Called by the shell each frame. Cheap (an `Arc` clone under a lock).
pub fn set_context(ctx: egui::Context) {
    if let Ok(mut slot) = context_slot().lock() {
        *slot = Some(ctx);
    }
}

fn with_context<T>(f: impl FnOnce(&egui::Context) -> T) -> Option<T> {
    let slot = context_slot().lock().ok()?;
    slot.as_ref().map(f)
}

/// Servo asks for a redraw through this; we turn it into an egui repaint.
struct Waker;

impl EventLoopWaker for Waker {
    fn clone_box(&self) -> Box<dyn EventLoopWaker> {
        Box::new(Waker)
    }

    fn wake(&self) {
        let _ = with_context(|ctx| ctx.request_repaint());
    }
}

/// Servo's per-webview delegate. Only frame readiness matters here; the rest of
/// the lifecycle is driven by the manager.
struct Delegate {
    frame_ready: Cell<bool>,
}

impl WebViewDelegate for Delegate {
    fn notify_new_frame_ready(&self, _webview: WebView) {
        self.frame_ready.set(true);
        let _ = with_context(|ctx| ctx.request_repaint());
    }
}

pub struct LinuxWebEmbedBackend {
    servo: Option<Rc<Servo>>,
    context: Option<Rc<SoftwareRenderingContext>>,
    webview: Option<WebView>,
    delegate: Option<Rc<Delegate>>,
    texture: Option<TextureHandle>,
    size: (u32, u32),
    pixels_per_point: f32,
}

impl LinuxWebEmbedBackend {
    pub fn new() -> Self {
        Self {
            servo: None,
            context: None,
            webview: None,
            delegate: None,
            texture: None,
            size: (0, 0),
            pixels_per_point: 1.0,
        }
    }

    fn ensure_servo(&mut self) -> Rc<Servo> {
        if let Some(servo) = &self.servo {
            return servo.clone();
        }
        // Servo's network stack needs a crypto provider. Papo or another
        // dependency may have installed one already; a second install is a
        // harmless error.
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        let servo = Rc::new(
            ServoBuilder::default()
                .event_loop_waker(Box::new(Waker))
                .build(),
        );
        self.servo = Some(servo.clone());
        servo
    }

    fn active(&self) -> Option<(&Rc<Servo>, &Rc<SoftwareRenderingContext>, &WebView, &Rc<Delegate>)> {
        Some((
            self.servo.as_ref()?,
            self.context.as_ref()?,
            self.webview.as_ref()?,
            self.delegate.as_ref()?,
        ))
    }

    fn upload(&mut self, image: &image::RgbaImage) {
        let size = [image.width() as usize, image.height() as usize];
        let color = ColorImage::from_rgba_unmultiplied(size, image.as_raw());
        if let Some(texture) = &mut self.texture {
            texture.set(color, TextureOptions::LINEAR);
            return;
        }
        self.texture = with_context(|ctx| {
            Some(ctx.load_texture("webembed", color, TextureOptions::LINEAR))
        })
        .flatten();
    }
}

impl Default for LinuxWebEmbedBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WebEmbedBackend for LinuxWebEmbedBackend {
    fn create(&mut self, _id: &str, url: &str) -> Result<(), String> {
        let servo = self.ensure_servo();
        let parsed = url::Url::parse(url).map_err(|error| error.to_string())?;

        let context = Rc::new(
            SoftwareRenderingContext::new(dpi::PhysicalSize::new(16, 16))
                .map_err(|error| format!("{error:?}"))?,
        );
        let delegate = Rc::new(Delegate {
            frame_ready: Cell::new(false),
        });
        let rendering: Rc<dyn RenderingContext> = context.clone();
        let webview = WebViewBuilder::new(&servo, rendering)
            .url(parsed)
            .delegate(delegate.clone())
            .build();

        self.servo = Some(servo);
        self.context = Some(context);
        self.delegate = Some(delegate);
        self.webview = Some(webview);
        self.texture = None;
        self.size = (0, 0);
        Ok(())
    }

    fn present(&mut self, _id: &str, viewport: &EmbedViewport) {
        let Some((servo, context, webview, delegate)) = self.active() else {
            return;
        };
        let servo = servo.clone();
        let context = context.clone();
        let webview = webview.clone();
        let frame_ready = delegate.frame_ready.clone();

        let ppp = viewport.pixels_per_point.max(0.1);
        let width = (viewport.rect.width() * ppp).round().max(1.0) as u32;
        let height = (viewport.rect.height() * ppp).round().max(1.0) as u32;

        if (width, height) != self.size {
            let size = dpi::PhysicalSize::new(width, height);
            context.resize(size);
            webview.resize(size);
            self.size = (width, height);
        }
        if (ppp - self.pixels_per_point).abs() > f32::EPSILON {
            webview.set_hidpi_scale_factor(Scale::<f32, DeviceIndependentPixel, DevicePixel>::new(ppp));
            self.pixels_per_point = ppp;
        }

        servo.spin_event_loop();

        if frame_ready.replace(false) {
            webview.paint();
            let rect = DeviceIntRect::from_origin_and_size(
                DeviceIntPoint::new(0, 0),
                DeviceIntSize::new(width as i32, height as i32),
            );
            if let Some(image) = context.read_to_image(rect) {
                self.upload(&image);
            }
        }
    }

    fn suspend(&mut self, _id: &str) {
        if let Some(webview) = &self.webview {
            webview.set_throttled(true);
        }
    }

    fn resume(&mut self, _id: &str) {
        if let Some(webview) = &self.webview {
            webview.set_throttled(false);
        }
    }

    fn destroy(&mut self, _id: &str) {
        self.webview = None;
        self.delegate = None;
        self.texture = None;
        self.size = (0, 0);
    }

    fn poll_events(&mut self) -> Vec<WebEmbedEvent> {
        Vec::new()
    }

    fn frame(&mut self) -> Option<TextureHandle> {
        self.texture.clone()
    }

    fn input(&mut self, _id: &str, input: WebEmbedInput) {
        let Some((servo, _context, webview, _delegate)) = self.active() else {
            return;
        };
        let servo = servo.clone();
        let webview = webview.clone();
        let ppp = self.pixels_per_point;
        let point = |x: f32, y: f32| WebViewPoint::from(DevicePoint::new(x * ppp, y * ppp));

        let event = match input {
            WebEmbedInput::Move { x, y } => InputEvent::MouseMove(MouseMoveEvent::new(point(x, y))),
            WebEmbedInput::Down { x, y, button } => InputEvent::MouseButton(MouseButtonEvent::new(
                MouseButtonAction::Down,
                servo_button(button),
                point(x, y),
            )),
            WebEmbedInput::Up { x, y, button } => InputEvent::MouseButton(MouseButtonEvent::new(
                MouseButtonAction::Up,
                servo_button(button),
                point(x, y),
            )),
            WebEmbedInput::Wheel { x, y, delta_y } => InputEvent::Wheel(WheelEvent::new(
                WheelDelta {
                    x: 0.0,
                    y: delta_y as f64,
                    z: 0.0,
                    mode: WheelMode::DeltaPixel,
                },
                point(x, y),
            )),
            WebEmbedInput::Leave => InputEvent::MouseLeftViewport(MouseLeftViewportEvent {
                focus_moving_to_another_iframe: false,
            }),
        };

        webview.notify_input_event(event);
        servo.spin_event_loop();
    }
}

fn servo_button(button: WebEmbedButton) -> MouseButton {
    match button {
        WebEmbedButton::Left => MouseButton::Left,
    }
}
