//! Linux WebEmbed backend powered by WPE WebKit.
//!
//! WPE renders into DMA-BUFs. Desktop rendering is WGPU-only, so the Linux
//! bridge maps the linear WPE buffer, converts its DRM pixel layout to RGBA,
//! and uploads it into one persistent WGPU texture registered with egui.
//!
//! This is the correctness path. A future zero-copy importer can replace the
//! mmap/upload step without changing the WebEmbed/egui contract.

#![cfg(target_os = "linux")]

use std::os::fd::AsRawFd as _;
use std::time::Instant;

use eframe::wgpu;

use crate::platform::linux_wpe::{Frame as WpeFrame, Page, PageEvent, Runtime, RuntimePaths};
use crate::webembed::{
    EmbedViewport, WebEmbedBackend, WebEmbedButton, WebEmbedEvent, WebEmbedInput,
};

const DRM_FORMAT_MOD_LINEAR: u64 = 0;

const fn fourcc(a: u8, b: u8, c: u8, d: u8) -> u32 {
    (a as u32) | ((b as u32) << 8) | ((c as u32) << 16) | ((d as u32) << 24)
}

const DRM_FORMAT_XRGB8888: u32 = fourcc(b'X', b'R', b'2', b'4');
const DRM_FORMAT_ARGB8888: u32 = fourcc(b'A', b'R', b'2', b'4');
const DRM_FORMAT_XBGR8888: u32 = fourcc(b'X', b'B', b'2', b'4');
const DRM_FORMAT_ABGR8888: u32 = fourcc(b'A', b'B', b'2', b'4');

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

fn map_frame_rgba(frame: &WpeFrame) -> Result<Vec<u8>, String> {
    if frame.modifier != DRM_FORMAT_MOD_LINEAR {
        return Err(format!(
            "WPE entregou DMA-BUF com modifier não-linear 0x{:x}",
            frame.modifier
        ));
    }

    let row_bytes = (frame.width as usize)
        .checked_mul(4)
        .ok_or_else(|| "largura WPE excede usize".to_owned())?;
    if (frame.stride as usize) < row_bytes {
        return Err(format!(
            "stride WPE inválido: {} para {} px",
            frame.stride, frame.width
        ));
    }

    let body_len = (frame.stride as usize)
        .checked_mul(frame.height as usize)
        .ok_or_else(|| "frame WPE excede usize".to_owned())?;
    let map_len = (frame.offset as usize)
        .checked_add(body_len)
        .ok_or_else(|| "offset WPE excede usize".to_owned())?;
    if map_len == 0 {
        return Err("frame WPE vazio".to_owned());
    }

    // SAFETY: the frame owns a live DMA-BUF fd for the duration of this call.
    // We map it read-only, bounds-check all row accesses below, then unmap it
    // before returning the WPE lease.
    let mapped = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            map_len,
            libc::PROT_READ,
            libc::MAP_SHARED,
            frame.plane.as_raw_fd(),
            0,
        )
    };
    if mapped == libc::MAP_FAILED {
        return Err(format!(
            "mmap do DMA-BUF WPE falhou: {}",
            std::io::Error::last_os_error()
        ));
    }

    struct Mapping {
        ptr: *mut libc::c_void,
        len: usize,
    }
    impl Drop for Mapping {
        fn drop(&mut self) {
            // SAFETY: this pair is exactly the successful mmap above.
            unsafe {
                libc::munmap(self.ptr, self.len);
            }
        }
    }
    let mapping = Mapping {
        ptr: mapped,
        len: map_len,
    };

    // SAFETY: map_len bytes are live until mapping is dropped.
    let bytes = unsafe { std::slice::from_raw_parts(mapping.ptr.cast::<u8>(), mapping.len) };
    let start = frame.offset as usize;
    let mut rgba = vec![
        0_u8;
        row_bytes
            .checked_mul(frame.height as usize)
            .ok_or_else(|| "frame RGBA excede usize".to_owned())?
    ];

    for y in 0..frame.height as usize {
        let source_start = start + y * frame.stride as usize;
        let source_end = source_start + row_bytes;
        let source = bytes
            .get(source_start..source_end)
            .ok_or_else(|| "linha WPE fora do DMA-BUF mapeado".to_owned())?;
        let destination = &mut rgba[y * row_bytes..(y + 1) * row_bytes];

        match frame.format {
            // Little-endian DRM XRGB/ARGB memory is B,G,R,X/A.
            DRM_FORMAT_XRGB8888 | DRM_FORMAT_ARGB8888 => {
                for pixel in 0..frame.width as usize {
                    let offset = pixel * 4;
                    destination[offset] = source[offset + 2];
                    destination[offset + 1] = source[offset + 1];
                    destination[offset + 2] = source[offset];
                    destination[offset + 3] = if frame.format == DRM_FORMAT_ARGB8888 {
                        source[offset + 3]
                    } else {
                        0xff
                    };
                }
            }
            // Little-endian DRM XBGR/ABGR memory is R,G,B,X/A.
            DRM_FORMAT_XBGR8888 | DRM_FORMAT_ABGR8888 => {
                for pixel in 0..frame.width as usize {
                    let offset = pixel * 4;
                    destination[offset] = source[offset];
                    destination[offset + 1] = source[offset + 1];
                    destination[offset + 2] = source[offset + 2];
                    destination[offset + 3] = if frame.format == DRM_FORMAT_ABGR8888 {
                        source[offset + 3]
                    } else {
                        0xff
                    };
                }
            }
            other => {
                return Err(format!("formato DRM WPE não suportado: 0x{other:08x}"));
            }
        }
    }

    Ok(rgba)
}

pub struct LinuxWebEmbedBackend {
    runtime: Option<Runtime>,
    page: Option<Page>,
    current_id: Option<String>,
    events: Vec<WebEmbedEvent>,
    egui: Option<egui::Context>,
    pending_frame: Option<WpeFrame>,
    native_texture: Option<wgpu::Texture>,
    texture_id: Option<egui::TextureId>,
    texture_size: (u32, u32),
    started: Instant,
    last_pointer: Option<(f32, f32)>,
    pointer_modifiers: u32,
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
            native_texture: None,
            texture_id: None,
            texture_size: (0, 0),
            started: Instant::now(),
            last_pointer: None,
            pointer_modifiers: 0,
        }
    }

    fn ensure_runtime(&mut self) -> Result<Runtime, String> {
        if let Some(runtime) = &self.runtime {
            return Ok(runtime.clone());
        }
        let paths = RuntimePaths::discover()?;
        let runtime = Runtime::open(&paths)?;
        log::info!("webembed(wpe): runtime em {}", paths.root().display());
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
        self.pointer_modifiers = 0;
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
        page.resize(
            width,
            height,
            f64::from(viewport.pixels_per_point.max(0.1)),
        );
        self.pump_page();
        if let Some(ctx) = &self.egui {
            ctx.request_repaint();
        }
    }

    fn suspend(&mut self, id: &str) {
        if self.current_id.as_deref() == Some(id)
            && let Some(page) = &self.page
        {
            page.set_focus(false);
        }
    }

    fn resume(&mut self, id: &str) {
        if self.current_id.as_deref() == Some(id)
            && let Some(ctx) = &self.egui
        {
            ctx.request_repaint();
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
        self.pointer_modifiers = 0;
        // Keep the WGPU texture registered and reuse it when dimensions match.
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

        let Some(render_state) = frame.wgpu_render_state().cloned() else {
            log::warn!("webembed(wpe): eframe sem WGPU");
            self.pending_frame = Some(pending);
            return;
        };

        let rgba = match map_frame_rgba(&pending) {
            Ok(rgba) => rgba,
            Err(error) => {
                log::error!("webembed(wpe): import DMA-BUF falhou: {error}");
                return;
            }
        };

        let size_changed = self.texture_size != (pending.width, pending.height);
        if self.native_texture.is_none() || size_changed {
            let texture = render_state.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("papo_wpe_webembed"),
                size: wgpu::Extent3d {
                    width: pending.width,
                    height: pending.height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[wgpu::TextureFormat::Rgba8Unorm],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut renderer = render_state.renderer.write();
            if let Some(texture_id) = self.texture_id {
                renderer.update_egui_texture_from_wgpu_texture(
                    &render_state.device,
                    &view,
                    wgpu::FilterMode::Linear,
                    texture_id,
                );
            } else {
                self.texture_id = Some(renderer.register_native_texture(
                    &render_state.device,
                    &view,
                    wgpu::FilterMode::Linear,
                ));
            }
            drop(renderer);
            self.native_texture = Some(texture);
        }

        let Some(texture) = self.native_texture.as_ref() else {
            return;
        };
        render_state.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(pending.width * 4),
                rows_per_image: Some(pending.height),
            },
            wgpu::Extent3d {
                width: pending.width,
                height: pending.height,
                depth_or_array_layers: 1,
            },
        );

        self.texture_size = (pending.width, pending.height);
        pending.release();
        if let Some(ctx) = &self.egui {
            ctx.request_repaint();
        }
    }

    fn texture_id(&self) -> Option<egui::TextureId> {
        self.texture_id
    }

    fn texture_size(&self) -> Option<(u32, u32)> {
        (self.texture_size.0 > 0 && self.texture_size.1 > 0).then_some(self.texture_size)
    }

    fn is_playing(&self, id: &str) -> Option<bool> {
        if self.current_id.as_deref() != Some(id) {
            return None;
        }
        self.page.as_ref().map(Page::is_playing_audio)
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
                    self.pointer_modifiers,
                    time,
                );
            }
            WebEmbedInput::Down { x, y, button: WebEmbedButton::Left } => {
                page.set_focus(true);
                self.pointer_modifiers |= 1 << 8;
                page.pointer_button(
                    true,
                    f64::from(x),
                    f64::from(y),
                    self.pointer_modifiers,
                    time,
                );
            }
            WebEmbedInput::Up { x, y, button: WebEmbedButton::Left } => {
                self.pointer_modifiers &= !(1 << 8);
                page.pointer_button(
                    false,
                    f64::from(x),
                    f64::from(y),
                    self.pointer_modifiers,
                    time,
                );
            }
            WebEmbedInput::Wheel { x, y, delta_y } => {
                page.scroll(
                    f64::from(x),
                    f64::from(y),
                    0.0,
                    f64::from(-delta_y),
                    self.pointer_modifiers,
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
