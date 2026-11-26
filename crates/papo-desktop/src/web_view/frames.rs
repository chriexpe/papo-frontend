//! A page's frames into Rune (`docs/papo-desktop.md` §5, the spike's path): each buffer WPE
//! finishes is a linear ARGB8888 dmabuf, pushed to the page's `NativeSurface`, which copies it
//! on Rune's GPU the first time it draws it. Owns the buffers' identity (one
//! `Arc<DmabufBuffer>` per WPE buffer, so Rune imports each once and not every frame), the
//! wait for WebKit's rendering fence (Rune reads only finished frames) and the hand-back: a
//! buffer returns to WPE once Rune has read it, or once a newer frame replaced it unread, so
//! WebKit renders the next frame only after the last one was used. A box Rune doesn't draw
//! (scrolled away, the window hidden) therefore stops the page's rendering at one frame.
//!
//! Safety: runs on the WPE thread with the buffers its view passes; the hand-back holds its
//! own references to the view and buffer and runs back on that thread.

#![expect(unsafe_code, reason = "reading WPE's dmabuf buffers")]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::os::fd::{BorrowedFd, FromRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use glib_sys::GFALSE;
use rune::NativeSurface;
use rune::native_surface::linux::{DRM_FORMAT_ARGB8888, Dmabuf, DmabufBuffer, DmabufPlane};

use super::glib::Context;
use super::library::{Object, Wpe};

/// How long a frame waits for WebKit's GPU work before it is shown anyway.
const FENCE_WAIT_MS: i32 = 1000;

/// A page's frames.
pub(super) struct Frames {
    wpe: &'static Wpe,
    context: Context,
    surface: NativeSurface,
    /// The page is open: a hand-back after it closed only lets its references go.
    alive: Arc<AtomicBool>,
    /// Each WPE buffer, imported once, by its address; WPE says when the set changes.
    buffers: RefCell<HashMap<usize, Arc<DmabufBuffer>>>,
    next_frame: Cell<u64>,
}

/// Why a frame couldn't be shown.
#[derive(Debug)]
pub(super) struct FrameFailure(pub(super) String);

impl Frames {
    pub(super) fn new(wpe: &'static Wpe, context: Context, surface: NativeSurface) -> Frames {
        Frames {
            wpe,
            context,
            surface,
            alive: Arc::new(AtomicBool::new(true)),
            buffers: RefCell::new(HashMap::new()),
            next_frame: Cell::new(1),
        }
    }

    /// Shows `buffer`, which WebKit rendered for `view`; it returns to WPE once Rune has read
    /// it. Returns whether it was the page's first frame.
    pub(super) fn render(&self, view: Object, buffer: Object) -> Result<bool, FrameFailure> {
        let back = HandBack::new(
            self.wpe,
            self.context,
            view,
            buffer,
            Arc::clone(&self.alive),
        );
        let imported = self.import(buffer)?;
        self.wait_for_rendering(buffer);
        let id = self.next_frame.get();
        self.next_frame.set(id + 1);
        let frame = Dmabuf::new(imported).on_read(move |_read| drop(back));
        self.surface.push_dmabuf(frame, id).map_err(FrameFailure)?;
        Ok(id == 1)
    }

    /// WPE changed its set of buffers: the old ones are let go (Rune forgets an import once
    /// nothing holds its buffer).
    pub(super) fn forget(&self) {
        self.buffers.borrow_mut().clear();
    }

    /// The page closes: no frame is drawn again and hand-backs only drop their references.
    pub(super) fn close(&self) {
        self.alive.store(false, Ordering::Release);
        self.surface.clear();
        self.forget();
    }

    /// `buffer` as Rune's buffer, made the first time it is seen.
    fn import(&self, buffer: Object) -> Result<Arc<DmabufBuffer>, FrameFailure> {
        let key = buffer as usize;
        if let Some(known) = self.buffers.borrow().get(&key) {
            return Ok(Arc::clone(known));
        }
        let wpe = self.wpe;
        // SAFETY: `buffer` is the live WPEBuffer WPE passed to the view; a dmabuf's getters
        // are read only after the type check, and its file descriptor is duplicated, not
        // taken.
        let (size, fourcc, plane) = unsafe {
            let is_dmabuf = gobject_sys::g_type_check_instance_is_a(
                buffer.cast(),
                (wpe.wpe_buffer_dma_buf_get_type)(),
            );
            if is_dmabuf == GFALSE {
                return Err(FrameFailure(
                    "WebKit rendered into a buffer that isn't a dmabuf".into(),
                ));
            }
            if (wpe.wpe_buffer_dma_buf_get_n_planes)(buffer) != 1 {
                return Err(FrameFailure(
                    "WebKit rendered a frame of several planes".into(),
                ));
            }
            let width = u32::try_from((wpe.wpe_buffer_get_width)(buffer)).unwrap_or(0);
            let height = u32::try_from((wpe.wpe_buffer_get_height)(buffer)).unwrap_or(0);
            let fd = (wpe.wpe_buffer_dma_buf_get_fd)(buffer, 0);
            if fd < 0 {
                return Err(FrameFailure("a dmabuf without its file".into()));
            }
            let owned = BorrowedFd::borrow_raw(fd)
                .try_clone_to_owned()
                .map_err(|error| FrameFailure(format!("duplicating a dmabuf: {error}")))?;
            let plane = DmabufPlane {
                fd: owned,
                offset: (wpe.wpe_buffer_dma_buf_get_offset)(buffer, 0),
                stride: (wpe.wpe_buffer_dma_buf_get_stride)(buffer, 0),
                modifier: (wpe.wpe_buffer_dma_buf_get_modifier)(buffer),
            };
            (
                [width, height],
                (wpe.wpe_buffer_dma_buf_get_format)(buffer),
                plane,
            )
        };
        if fourcc != DRM_FORMAT_ARGB8888 {
            return Err(FrameFailure(format!(
                "WebKit rendered format {fourcc:#x}, not ARGB8888"
            )));
        }
        let imported = Arc::new(DmabufBuffer::new(plane, size, fourcc).map_err(FrameFailure)?);
        self.buffers.borrow_mut().insert(key, Arc::clone(&imported));
        Ok(imported)
    }

    /// Waits until WebKit's GPU has finished `buffer`, when it says how to know.
    fn wait_for_rendering(&self, buffer: Object) {
        // SAFETY: the fence, when there is one, is a sync file this call now owns.
        let fence = unsafe { (self.wpe.wpe_buffer_take_rendering_fence)(buffer) };
        if fence < 0 {
            return;
        }
        // SAFETY: `fence` was just handed over, so it is closed exactly once, here.
        let fence = unsafe { OwnedFd::from_raw_fd(fence) };
        let mut poll = glib_sys::GPollFD {
            fd: std::os::fd::AsRawFd::as_raw_fd(&fence),
            events: u16::try_from(glib_sys::G_IO_IN).unwrap_or(1),
            revents: 0,
        };
        // SAFETY: one poll record, valid for the call.
        unsafe { glib_sys::g_poll(&raw mut poll, 1, FENCE_WAIT_MS) };
    }
}

/// Gives a buffer back to WPE when dropped: once Rune read its frame, or once the frame was
/// replaced or let go unread.
struct HandBack {
    wpe: &'static Wpe,
    context: Context,
    view: Ref,
    buffer: Ref,
    alive: Arc<AtomicBool>,
}

/// A reference to a GObject, owned by a hand-back.
struct Ref(Object);

// SAFETY: the reference is only taken and released on the WPE thread; other threads only move
// it there.
unsafe impl Send for Ref {}

impl HandBack {
    fn new(
        wpe: &'static Wpe,
        context: Context,
        view: Object,
        buffer: Object,
        alive: Arc<AtomicBool>,
    ) -> HandBack {
        // SAFETY: live objects WPE passed; each reference is released by `give_back`.
        unsafe {
            gobject_sys::g_object_ref(view.cast());
            gobject_sys::g_object_ref(buffer.cast());
        }
        HandBack {
            wpe,
            context,
            view: Ref(view),
            buffer: Ref(buffer),
            alive,
        }
    }
}

impl Drop for HandBack {
    fn drop(&mut self) {
        let wpe = self.wpe;
        let view = Ref(self.view.0);
        let buffer = Ref(self.buffer.0);
        let alive = Arc::clone(&self.alive);
        self.context
            .invoke(move || give_back(wpe, &view, &buffer, &alive));
    }
}

fn give_back(wpe: &Wpe, view: &Ref, buffer: &Ref, alive: &AtomicBool) {
    // SAFETY: on the WPE thread, with the references `HandBack::new` took.
    unsafe {
        if alive.load(Ordering::Acquire) {
            (wpe.wpe_view_buffer_rendered)(view.0, buffer.0);
            (wpe.wpe_view_buffer_released)(view.0, buffer.0);
        }
        gobject_sys::g_object_unref(buffer.0.cast());
        gobject_sys::g_object_unref(view.0.cast());
    }
}
