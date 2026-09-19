//! Aviso ao KWin de onde está o menu, no Wayland.
//!
//! No X11 isso é feito por propriedades da janela (ver `global_menu`); no
//! Wayland existe um protocolo próprio do Plasma que liga uma `wl_surface` ao
//! endereço D-Bus do menu.

use std::ffi::c_void;
use std::ptr::NonNull;

use wayland_backend::sys::client::{Backend, ObjectId};
use wayland_client::protocol::{
    wl_registry::{self, WlRegistry},
    wl_surface::WlSurface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_plasma::appmenu::client::{
    org_kde_kwin_appmenu::OrgKdeKwinAppmenu, org_kde_kwin_appmenu_manager::OrgKdeKwinAppmenuManager,
};

#[derive(Default)]
struct Globals {
    manager: Option<OrgKdeKwinAppmenuManager>,
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
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "org_kde_kwin_appmenu_manager" {
                state.manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
        }
    }
}

delegate_noop!(Globals: ignore OrgKdeKwinAppmenuManager);
delegate_noop!(Globals: ignore OrgKdeKwinAppmenu);

/// Mantém vivo o objeto de menu da superfície.
pub struct AppMenuSurface {
    conn: Connection,
    _queue: EventQueue<Globals>,
    appmenu: OrgKdeKwinAppmenu,
}

impl AppMenuSurface {
    /// # Safety
    /// `display` e `surface` precisam ser ponteiros válidos da janela em uso.
    pub unsafe fn attach(
        display: NonNull<c_void>,
        surface: NonNull<c_void>,
        service: &str,
        path: &str,
    ) -> Option<Self> {
        let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
        let conn = Connection::from_backend(backend);

        let mut queue: EventQueue<Globals> = conn.new_event_queue();
        let qh = queue.handle();
        let _registry = conn.display().get_registry(&qh, ());

        let mut globals = Globals::default();
        queue.roundtrip(&mut globals).ok()?;
        let manager = globals.manager?;

        let surface_id =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast()) }.ok()?;
        let surface = WlSurface::from_id(&conn, surface_id).ok()?;

        let appmenu = manager.create(&surface, &qh, ());
        appmenu.set_address(service.to_owned(), path.to_owned());
        let _ = conn.flush();
        log::info!("menu global ligado à superfície wayland");

        Some(Self {
            conn,
            _queue: queue,
            appmenu,
        })
    }
}

impl Drop for AppMenuSurface {
    fn drop(&mut self) {
        self.appmenu.release();
        let _ = self.conn.flush();
    }
}
