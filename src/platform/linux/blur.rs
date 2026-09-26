//! Desfoque real por trás das barras translúcidas, via protocolo do KWin.
//!
//! O eframe/winit já mantém uma conexão Wayland; em vez de abrir outra,
//! reaproveitamos o mesmo `wl_display` e a mesma `wl_surface` por ponteiro e
//! criamos só uma fila de eventos própria para o `org_kde_kwin_blur`.

use std::ffi::c_void;
use std::ptr::NonNull;

use wayland_backend::sys::client::{Backend, ObjectId};
use wayland_client::protocol::{
    wl_compositor::WlCompositor,
    wl_region::WlRegion,
    wl_registry::{self, WlRegistry},
    wl_surface::WlSurface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_plasma::blur::client::{
    org_kde_kwin_blur::OrgKdeKwinBlur, org_kde_kwin_blur_manager::OrgKdeKwinBlurManager,
};

/// Retângulo em coordenadas da superfície (pixels lógicos).
pub type Region = (i32, i32, i32, i32);

#[derive(Default)]
struct Globals {
    compositor: Option<WlCompositor>,
    blur_manager: Option<OrgKdeKwinBlurManager>,
}

impl Dispatch<WlRegistry, ()> for Globals {
    fn event(
        state: &mut Self,
        registry: &WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        else {
            return;
        };
        match interface.as_str() {
            "wl_compositor" => {
                state.compositor = Some(registry.bind(name, version.min(4), qh, ()));
            }
            "org_kde_kwin_blur_manager" => {
                state.blur_manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
            _ => {}
        }
    }
}

delegate_noop!(Globals: ignore WlCompositor);
delegate_noop!(Globals: ignore WlRegion);
delegate_noop!(Globals: ignore OrgKdeKwinBlurManager);
delegate_noop!(Globals: ignore OrgKdeKwinBlur);

pub struct BlurSurface {
    conn: Connection,
    queue: EventQueue<Globals>,
    globals: Globals,
    blur: OrgKdeKwinBlur,
    current: Vec<Region>,
}

impl BlurSurface {
    /// # Safety
    /// `display` e `surface` precisam ser ponteiros vivos de `wl_display` e
    /// `wl_surface` da janela em uso.
    pub unsafe fn new(display: NonNull<c_void>, surface: NonNull<c_void>) -> Option<Self> {
        let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
        let conn = Connection::from_backend(backend);

        let mut queue: EventQueue<Globals> = conn.new_event_queue();
        let qh = queue.handle();
        let _registry = conn.display().get_registry(&qh, ());

        let mut globals = Globals::default();
        queue.roundtrip(&mut globals).ok()?;

        let manager = globals.blur_manager.clone()?;
        let surface_id =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast()) }.ok()?;
        let surface = WlSurface::from_id(&conn, surface_id).ok()?;

        let blur = manager.create(&surface, &qh, ());
        log::info!("desfoque do KWin ativo");

        Some(Self {
            conn,
            queue,
            globals,
            blur,
            current: Vec::new(),
        })
    }

    /// Define quais partes da janela o compositor deve desfocar.
    pub fn set_regions(&mut self, regions: &[Region]) {
        if self.current == regions {
            return;
        }
        let Some(compositor) = self.globals.compositor.clone() else {
            return;
        };

        let qh = self.queue.handle();
        let region = compositor.create_region(&qh, ());
        for (x, y, w, h) in regions {
            region.add(*x, *y, *w, *h);
        }
        self.blur.set_region(Some(&region));
        self.blur.commit();
        region.destroy();
        let _ = self.conn.flush();

        self.current = regions.to_vec();
    }
}

impl Drop for BlurSurface {
    fn drop(&mut self) {
        self.blur.release();
        let _ = self.conn.flush();
    }
}
