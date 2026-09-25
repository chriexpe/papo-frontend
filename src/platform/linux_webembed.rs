//! Linux WebEmbed backend powered by WPE WebKit.
//!
//! WPE renders into DMA-BUFs. Papo imports those buffers into the OpenGL
//! context eframe already owns, GPU-copies them into one persistent texture,
//! then returns the WPE buffer. No second GL context is ever made current.

#![cfg(target_os = "linux")]

use std::ffi::{CString, c_char, c_void};
use std::os::fd::AsRawFd as _;
use std::time::Instant;

use eframe::glow::{self, HasContext as _};

use crate::platform::linux_wpe::{Frame as WpeFrame, Page, PageEvent, Runtime, RuntimePaths};
use crate::webembed::{
    EmbedViewport, WebEmbedBackend, WebEmbedButton, WebEmbedEvent, WebEmbedInput,
};

const EGL_LINUX_DMA_BUF_EXT: u32 = 0x3270;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;
const EGL_LINUX_DRM_FOURCC_EXT: i32 = 0x3271;
const EGL_DMA_BUF_PLANE0_FD_EXT: i32 = 0x3272;
const EGL_DMA_BUF_PLANE0_OFFSET_EXT: i32 = 0x3273;
const EGL_DMA_BUF_PLANE0_PITCH_EXT: i32 = 0x3274;
const EGL_NONE: i32 = 0x3038;
const DRM_FORMAT_MOD_LINEAR: u64 = 0;

type EglDisplay = *mut c_void;
type EglImage = *mut c_void;
type EglGetCurrentDisplay = unsafe extern "C" fn() -> EglDisplay;
type EglGetProcAddress = unsafe extern "C" fn(*const c_char) -> *const c_void;
type EglCreateImageKhr = unsafe extern "C" fn(
    EglDisplay,
    *mut c_void,
    u32,
    *mut c_void,
    *const i32,
) -> EglImage;
type EglDestroyImageKhr = unsafe extern "C" fn(EglDisplay, EglImage) -> u32;
type EglGetError = unsafe extern "C" fn() -> u32;
type GlEglImageTargetTexture2dOes = unsafe extern "C" fn(u32, *mut c_void);

struct EglDmaBuf {
    _library: libloading::Library,
    get_current_display: EglGetCurrentDisplay,
    create_image: EglCreateImageKhr,
    destroy_image: EglDestroyImageKhr,
    get_error: EglGetError,
    image_target_texture: GlEglImageTargetTexture2dOes,
}

impl EglDmaBuf {
    fn load() -> Result<Self, String> {
        // SAFETY: libEGL stays mapped in this struct for all resolved pointers.
        let library = unsafe { libloading::Library::new("libEGL.so.1") }
            .map_err(|error| format!("libEGL.so.1 indisponível: {error}"))?;

        unsafe fn symbol<T: Copy>(
            library: &libloading::Library,
            name: &[u8],
        ) -> Result<T, String> {
            // SAFETY: symbol type matches the EGL ABI.
            unsafe { library.get::<T>(name) }
                .map(|value| *value)
                .map_err(|error| format!("símbolo EGL ausente: {error}"))
        }

        // SAFETY: exact EGL function signatures.
        let get_current_display =
            unsafe { symbol::<EglGetCurrentDisplay>(&library, b"eglGetCurrentDisplay\0")? };
        let get_proc_address =
            unsafe { symbol::<EglGetProcAddress>(&library, b"eglGetProcAddress\0")? };
        let get_error = unsafe { symbol::<EglGetError>(&library, b"eglGetError\0")? };

        let create_image = unsafe {
            resolve_proc::<EglCreateImageKhr>(&library, get_proc_address, b"eglCreateImageKHR\0")?
        };
        let destroy_image = unsafe {
            resolve_proc::<EglDestroyImageKhr>(&library, get_proc_address, b"eglDestroyImageKHR\0")?
        };
        let image_target_texture = unsafe {
            resolve_proc::<GlEglImageTargetTexture2dOes>(
                &library,
                get_proc_address,
                b"glEGLImageTargetTexture2DOES\0",
            )?
        };

        Ok(Self {
            _library: library,
            get_current_display,
            create_image,
            destroy_image,
            get_error,
            image_target_texture,
        })
    }

    fn import_into(
        &self,
        gl: &glow::Context,
        source: &WpeFrame,
        destination: glow::Texture,
    ) -> Result<(), String> {
        if source.modifier != DRM_FORMAT_MOD_LINEAR {
            return Err(format!(
                "WPE entregou DMA-BUF com modifier não-linear 0x{:x}",
                source.modifier
            ));
        }

        // SAFETY: eframe has its host context current while App::ui runs.
        let display = unsafe { (self.get_current_display)() };
        if display.is_null() {
            return Err(
                "o contexto Glow atual não é EGL; importação WPE DMA-BUF indisponível".to_owned(),
            );
        }

        let attributes = [
            EGL_WIDTH,
            source.width as i32,
            EGL_HEIGHT,
            source.height as i32,
            EGL_LINUX_DRM_FOURCC_EXT,
            source.format as i32,
            EGL_DMA_BUF_PLANE0_FD_EXT,
            source.plane.as_raw_fd(),
            EGL_DMA_BUF_PLANE0_OFFSET_EXT,
            source.offset as i32,
            EGL_DMA_BUF_PLANE0_PITCH_EXT,
            source.stride as i32,
            EGL_NONE,
        ];

        // SAFETY: attributes reference the live DMA-BUF frame for this call.
        let image = unsafe {
            (self.create_image)(
                display,
                std::ptr::null_mut(),
                EGL_LINUX_DMA_BUF_EXT,
                std::ptr::null_mut(),
                attributes.as_ptr(),
            )
        };
        if image.is_null() {
            // SAFETY: plain EGL error query.
            let error = unsafe { (self.get_error)() };
            return Err(format!("eglCreateImageKHR(DMA-BUF) falhou: 0x{error:04x}"));
        }

        let result = unsafe {
            let imported = gl
                .create_texture()
                .map_err(|error| format!("falha ao criar textura temporária WPE: {error}"))?;
            gl.bind_texture(glow::TEXTURE_2D, Some(imported));
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );

            (self.image_target_texture)(glow::TEXTURE_2D, image);

            let read_fbo = gl
                .create_framebuffer()
                .map_err(|error| format!("falha ao criar FBO WPE: {error}"))?;
            gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(read_fbo));
            gl.framebuffer_texture_2d(
                glow::READ_FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(imported),
                0,
            );
            if gl.check_framebuffer_status(glow::READ_FRAMEBUFFER)
                != glow::FRAMEBUFFER_COMPLETE
            {
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
                gl.delete_framebuffer(read_fbo);
                gl.delete_texture(imported);
                Err("DMA-BUF importado não formou framebuffer completo".to_owned())
            } else {
                gl.read_buffer(glow::COLOR_ATTACHMENT0);
                gl.bind_texture(glow::TEXTURE_2D, Some(destination));
                gl.copy_tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA as i32,
                    0,
                    0,
                    source.width as i32,
                    source.height as i32,
                    0,
                );
                // First correctness pass: guarantee the GPU has finished
                // reading WPE's buffer before it returns to WebKit's pool.
                gl.finish();

                gl.bind_texture(glow::TEXTURE_2D, None);
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
                gl.delete_framebuffer(read_fbo);
                gl.delete_texture(imported);
                Ok(())
            }
        };

        // SAFETY: image belongs to this display and was created above.
        unsafe {
            (self.destroy_image)(display, image);
        }
        result
    }
}

unsafe fn resolve_proc<T: Copy>(
    library: &libloading::Library,
    get_proc_address: EglGetProcAddress,
    name: &[u8],
) -> Result<T, String> {
    if let Ok(symbol) = unsafe { library.get::<T>(name) } {
        return Ok(*symbol);
    }
    let name = CString::from_vec_with_nul(name.to_vec())
        .map_err(|_| "nome de símbolo EGL inválido".to_owned())?;
    // SAFETY: eglGetProcAddress accepts a NUL-terminated symbol name.
    let pointer = unsafe { get_proc_address(name.as_ptr()) };
    if pointer.is_null() {
        return Err(format!(
            "extensão EGL/GL ausente: {}",
            name.to_string_lossy()
        ));
    }
    // SAFETY: caller chose T to match the named function's ABI.
    Ok(unsafe { std::mem::transmute_copy::<*const c_void, T>(&pointer) })
}

fn fence_ready(frame: &WpeFrame) -> bool {
    let Some(fence) = frame.rendering_fence.as_ref() else {
        return true;
    };
    let mut pollfd = libc::pollfd {
        fd: fence.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one valid pollfd and zero timeout.
    unsafe { libc::poll(&mut pollfd, 1, 0) > 0 }
}

pub struct LinuxWebEmbedBackend {
    runtime: Option<Runtime>,
    page: Option<Page>,
    current_id: Option<String>,
    events: Vec<WebEmbedEvent>,
    egui: Option<egui::Context>,
    pending_frame: Option<WpeFrame>,
    egl: Option<Result<EglDmaBuf, String>>,
    native_texture: Option<glow::Texture>,
    texture_id: Option<egui::TextureId>,
    texture_size: (u32, u32),
    started: Instant,
    last_pointer: Option<(f32, f32)>,
}

impl LinuxWebEmbedBackend {
    pub fn new() -> Self {
        Self {
            runtime: None,
            page: None,
            current_id: None,
            events: Vec::new(),
            egui: None,
            pending_frame: None,
            egl: None,
            native_texture: None,
            texture_id: None,
            texture_size: (0, 0),
            started: Instant::now(),
            last_pointer: None,
        }
    }

    fn ensure_runtime(&mut self) -> Result<Runtime, String> {
        if let Some(runtime) = &self.runtime {
            return Ok(runtime.clone());
        }
        let paths = RuntimePaths::discover()?;
        let runtime = Runtime::open(&paths)?;
        log::info!(
            "webembed(wpe): runtime {} em {}",
            crate::platform::linux_wpe::WPE_WEBKIT_VERSION,
            paths.root().display()
        );
        self.runtime = Some(runtime.clone());
        Ok(runtime)
    }

    fn pump_page(&mut self) {
        let Some(page) = self.page.clone() else {
            return;
        };
        page.pump();
        let Some(id) = self.current_id.clone() else {
            return;
        };
        for event in page.take_events() {
            match event {
                PageEvent::LoadFailed(message) => {
                    log::warn!("webembed(wpe): load falhou: {message}");
                    self.events.push(WebEmbedEvent::Failed { id: id.clone() });
                }
                PageEvent::NavigationStarted(url) => {
                    log::debug!("webembed(wpe): navegação {url}");
                }
                PageEvent::Loaded => {
                    log::debug!("webembed(wpe): carregado");
                }
            }
        }
        if self.pending_frame.is_none() {
            self.pending_frame = page.take_frame();
        }
    }

    fn time_ms(&self) -> u32 {
        self.started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32
    }
}

impl Default for LinuxWebEmbedBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WebEmbedBackend for LinuxWebEmbedBackend {
    fn create(&mut self, id: &str, url: &str) -> Result<(), String> {
        let runtime = self.ensure_runtime()?;
        let page = Page::new(runtime)?;
        if let Some(ctx) = self.egui.clone() {
            page.set_frame_waker(move || ctx.request_repaint());
        }
        page.resize(16, 16, 1.0);
        page.load(url)?;
        self.current_id = Some(id.to_owned());
        self.page = Some(page);
        self.pending_frame = None;
        self.last_pointer = None;
        Ok(())
    }

    fn present(&mut self, id: &str, viewport: &EmbedViewport) {
        if self.current_id.as_deref() != Some(id) {
            return;
        }
        let Some(page) = self.page.clone() else {
            return;
        };
        let width = viewport.rect.width().round().max(1.0) as u32;
        let height = viewport.rect.height().round().max(1.0) as u32;
        page.resize(width, height, f64::from(viewport.pixels_per_point.max(0.1)));
        self.pump_page();
        if let Some(ctx) = &self.egui {
            ctx.request_repaint();
        }
    }

    fn suspend(&mut self, id: &str) {
        if self.current_id.as_deref() == Some(id) {
            if let Some(page) = &self.page {
                page.set_focus(false);
            }
        }
    }

    fn resume(&mut self, id: &str) {
        if self.current_id.as_deref() == Some(id) {
            if let Some(ctx) = &self.egui {
                ctx.request_repaint();
            }
        }
    }

    fn destroy(&mut self, id: &str) {
        if self.current_id.as_deref() != Some(id) {
            return;
        }
        if let Some(page) = &self.page {
            page.stop();
            page.set_focus(false);
        }
        self.page = None;
        self.current_id = None;
        self.pending_frame = None;
        self.last_pointer = None;
        // Keep Papo's registered texture around and reuse it for the next
        // embed; eframe owns and eventually deletes it.
    }

    fn poll_events(&mut self) -> Vec<WebEmbedEvent> {
        self.pump_page();
        std::mem::take(&mut self.events)
    }

    fn set_context(&mut self, ctx: &egui::Context) {
        self.egui = Some(ctx.clone());
    }

    fn prepare_render(&mut self, frame: &mut eframe::Frame) {
        self.pump_page();
        let Some(pending) = self.pending_frame.take() else {
            return;
        };
        if !fence_ready(&pending) {
            self.pending_frame = Some(pending);
            if let Some(ctx) = &self.egui {
                ctx.request_repaint();
            }
            return;
        }

        let Some(gl) = frame.gl().cloned() else {
            log::warn!("webembed(wpe): eframe sem Glow");
            self.pending_frame = Some(pending);
            return;
        };

        if self.native_texture.is_none() {
            // SAFETY: this is Papo's current Glow context; no context switch.
            let texture = unsafe {
                match gl.create_texture() {
                    Ok(texture) => {
                        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
                        gl.tex_parameter_i32(
                            glow::TEXTURE_2D,
                            glow::TEXTURE_MIN_FILTER,
                            glow::LINEAR as i32,
                        );
                        gl.tex_parameter_i32(
                            glow::TEXTURE_2D,
                            glow::TEXTURE_MAG_FILTER,
                            glow::LINEAR as i32,
                        );
                        gl.bind_texture(glow::TEXTURE_2D, None);
                        texture
                    }
                    Err(error) => {
                        log::error!("webembed(wpe): textura Glow: {error}");
                        return;
                    }
                }
            };
            let texture_id = frame.register_native_glow_texture(texture);
            self.native_texture = Some(texture);
            self.texture_id = Some(texture_id);
        }

        let egl = self.egl.get_or_insert_with(EglDmaBuf::load);
        let Ok(egl) = egl else {
            log::error!("webembed(wpe): {}", egl.as_ref().unwrap_err());
            return;
        };
        let texture = self
            .native_texture
            .expect("registered WPE texture must have native handle");
        match egl.import_into(&gl, &pending, texture) {
            Ok(()) => {
                self.texture_size = (pending.width, pending.height);
                pending.release();
                if let Some(ctx) = &self.egui {
                    ctx.request_repaint();
                }
            }
            Err(error) => {
                log::error!("webembed(wpe): import DMA-BUF falhou: {error}");
                // Drop returns the lease to WPE.
            }
        }
    }

    fn texture_id(&self) -> Option<egui::TextureId> {
        self.texture_id
    }

    fn input(&mut self, id: &str, input: WebEmbedInput) {
        if self.current_id.as_deref() != Some(id) {
            return;
        }
        let Some(page) = self.page.clone() else {
            return;
        };
        let time = self.time_ms();
        match input {
            WebEmbedInput::Move { x, y } => {
                let (dx, dy) = self
                    .last_pointer
                    .map_or((0.0, 0.0), |(old_x, old_y)| (x - old_x, y - old_y));
                self.last_pointer = Some((x, y));
                page.pointer_move(
                    f64::from(x),
                    f64::from(y),
                    f64::from(dx),
                    f64::from(dy),
                    time,
                );
            }
            WebEmbedInput::Down { x, y, button: WebEmbedButton::Left } => {
                page.set_focus(true);
                page.pointer_button(true, f64::from(x), f64::from(y), time);
            }
            WebEmbedInput::Up { x, y, button: WebEmbedButton::Left } => {
                page.pointer_button(false, f64::from(x), f64::from(y), time);
            }
            WebEmbedInput::Wheel { x, y, delta_y } => {
                page.scroll(
                    f64::from(x),
                    f64::from(y),
                    0.0,
                    f64::from(-delta_y),
                    time,
                );
            }
            WebEmbedInput::Leave => {
                self.last_pointer = None;
            }
        }
        page.pump();
    }
}
