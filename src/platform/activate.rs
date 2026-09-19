//! Pedido de ativação da janela (xdg-activation).
//!
//! O winit não expõe `focus_window` no Wayland, mas o protocolo existe: pede
//! um token ao compositor e usa esse token para ativar a própria superfície.
//! O KWin pode atender levantando a janela ou apenas marcando-a como pedindo
//! atenção na barra de tarefas — nos dois casos é melhor que nada.

use std::ffi::c_void;
use std::ptr::NonNull;

use wayland_backend::sys::client::{Backend, ObjectId};
use wayland_client::protocol::{
    wl_registry::{self, WlRegistry},
    wl_surface::WlSurface,
};
use wayland_client::{delegate_noop, Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::{
    xdg_activation_token_v1::{self, XdgActivationTokenV1},
    xdg_activation_v1::XdgActivationV1,
};

#[derive(Default)]
struct State {
    manager: Option<XdgActivationV1>,
    token: Option<String>,
}

impl Dispatch<WlRegistry, ()> for State {
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
            if interface == "xdg_activation_v1" {
                state.manager = Some(registry.bind(name, version.min(1), qh, ()));
            }
        }
    }
}

impl Dispatch<XdgActivationTokenV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            state.token = Some(token);
        }
    }
}

delegate_noop!(State: ignore XdgActivationV1);

pub struct Activator {
    conn: Connection,
    queue: EventQueue<State>,
    state: State,
    surface: WlSurface,
}

impl Activator {
    /// # Safety
    /// Ponteiros válidos de `wl_display` e `wl_surface` da janela em uso.
    pub unsafe fn new(display: NonNull<c_void>, surface: NonNull<c_void>) -> Option<Self> {
        let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
        let conn = Connection::from_backend(backend);

        let mut queue: EventQueue<State> = conn.new_event_queue();
        let qh = queue.handle();
        let _registry = conn.display().get_registry(&qh, ());

        let mut state = State::default();
        queue.roundtrip(&mut state).ok()?;
        state.manager.as_ref()?;

        let surface_id =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.as_ptr().cast()) }.ok()?;
        let surface = WlSurface::from_id(&conn, surface_id).ok()?;

        Some(Self {
            conn,
            queue,
            state,
            surface,
        })
    }

    /// Pede ao compositor que traga a janela de volta.
    pub fn request(&mut self) {
        let Some(manager) = self.state.manager.clone() else {
            return;
        };
        let qh = self.queue.handle();

        self.state.token = None;
        let token = manager.get_activation_token(&qh, ());
        token.set_surface(&self.surface);
        token.set_app_id("papo".to_owned());
        token.commit();
        let _ = self.conn.flush();

        // O token chega pelo evento `done`.
        let mut state = std::mem::take(&mut self.state);
        let _ = self.queue.roundtrip(&mut state);
        let token = state.token.clone();
        self.state = state;

        match token {
            Some(token) => {
                manager.activate(token, &self.surface);
                let _ = self.conn.flush();
                log::debug!("ativação da janela pedida ao compositor");
            }
            None => log::debug!("compositor não devolveu token de ativação"),
        }
    }
}
