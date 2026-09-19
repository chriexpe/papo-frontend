//! Menu global: expõe a barra de menus em D-Bus (com.canonical.dbusmenu) e
//! diz ao KWin onde encontrá-la.
//!
//! O egui desenha a própria janela inteira e não tem barra de menus nativa,
//! então o menu vive só no painel do sistema. O serviço roda numa thread
//! própria com um runtime tokio de thread única; a interface troca mensagens
//! com ela por canais.

use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};
use zbus::object_server::SignalEmitter;
use zbus::{fdo, interface};
use zvariant::{Array, OwnedValue, Signature, Type, Value};

use super::menu::{MenuCommand, MenuKind, MenuModel, MenuNode};

pub const OBJECT_PATH: &str = "/MenuBar";
const ROOT_ID: i32 = 0;

// ---------------------------------------------------------------------------
// Estado achatado do menu
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Item {
    label: String,
    kind: MenuKind,
    enabled: bool,
    accel: Option<Vec<String>>,
    children: Vec<i32>,
    command: Option<MenuCommand>,
}

impl Item {
    fn root(children: Vec<i32>) -> Self {
        Self {
            label: String::new(),
            kind: MenuKind::Submenu,
            enabled: true,
            accel: None,
            children,
            command: None,
        }
    }
}

#[derive(Default)]
struct MenuState {
    items: HashMap<i32, Item>,
    revision: u32,
}

impl MenuState {
    fn load(&mut self, model: &MenuModel) {
        self.items.clear();
        let mut next_id = 1;
        let roots = model
            .roots
            .iter()
            .map(|node| self.insert(node, &mut next_id))
            .collect();
        self.items.insert(ROOT_ID, Item::root(roots));
        self.revision = self.revision.wrapping_add(1);
    }

    fn insert(&mut self, node: &MenuNode, next_id: &mut i32) -> i32 {
        let id = *next_id;
        *next_id += 1;
        let children = node
            .children
            .iter()
            .map(|child| self.insert(child, next_id))
            .collect();
        self.items.insert(
            id,
            Item {
                label: node.label.clone(),
                kind: node.kind.clone(),
                enabled: node.enabled,
                accel: node.accel.clone(),
                children,
                command: node.command,
            },
        );
        id
    }

    fn properties(&self, id: i32) -> HashMap<String, OwnedValue> {
        let mut props = HashMap::new();
        let Some(item) = self.items.get(&id) else {
            return props;
        };
        let mut put = |key: &str, value: Value<'static>| {
            if let Ok(value) = OwnedValue::try_from(value) {
                props.insert(key.to_owned(), value);
            }
        };

        if matches!(item.kind, MenuKind::Separator) {
            put("type", Value::from("separator"));
            return props;
        }

        // No DBusMenu o sublinhado marca a tecla de acesso; num rótulo comum
        // ele precisa vir dobrado.
        put("label", Value::from(item.label.replace('_', "__")));
        put("enabled", Value::from(item.enabled));
        put("visible", Value::from(true));

        if !item.children.is_empty() {
            put("children-display", Value::from("submenu"));
        }
        match item.kind {
            MenuKind::Checkbox { checked } => {
                put("toggle-type", Value::from("checkmark"));
                put("toggle-state", Value::from(i32::from(checked)));
            }
            MenuKind::Radio { selected } => {
                put("toggle-type", Value::from("radio"));
                put("toggle-state", Value::from(i32::from(selected)));
            }
            _ => {}
        }
        if let Some(accel) = &item.accel {
            let mut keys = Array::new(&Signature::static_array(&Signature::Str));
            let mut combo = Array::new(&Signature::Str);
            for key in accel {
                let _ = combo.append(Value::from(key.clone()));
            }
            let _ = keys.append(Value::Array(combo));
            put("shortcut", Value::Array(keys));
        }
        props
    }

    fn layout(&self, id: i32, depth: i32) -> LayoutItem {
        let children = if depth == 0 {
            Vec::new()
        } else {
            self.items
                .get(&id)
                .map(|item| {
                    item.children
                        .iter()
                        .map(|child| self.layout(*child, depth - 1))
                        .collect()
                })
                .unwrap_or_default()
        };
        LayoutItem {
            id,
            props: self.properties(id),
            children,
        }
    }
}

// ---------------------------------------------------------------------------
// Tipo recursivo do GetLayout: (ia{sv}av)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Type)]
#[zvariant(signature = "(ia{sv}av)")]
struct LayoutItem {
    id: i32,
    props: HashMap<String, OwnedValue>,
    children: Vec<LayoutItem>,
}

impl LayoutItem {
    fn as_value(&self) -> Value<'static> {
        // Um dicionário de assinatura a{sv} guarda variantes: cada valor
        // precisa entrar embrulhado, senão o append é recusado em silêncio.
        let mut dict = zvariant::Dict::new(&Signature::Str, &Signature::Variant);
        for (key, value) in &self.props {
            if let Err(error) = dict.append(
                Value::from(key.clone()),
                Value::Value(Box::new(Value::from(value.clone()))),
            ) {
                log::warn!("propriedade {key} do menu não entrou: {error}");
            }
        }
        let mut children = Array::new(&Signature::Variant);
        for child in &self.children {
            if let Err(error) = children.append(Value::Value(Box::new(child.as_value()))) {
                log::warn!("item do menu não entrou: {error}");
            }
        }
        Value::from(
            zvariant::StructureBuilder::new()
                .append_field(Value::from(self.id))
                .append_field(Value::Dict(dict))
                .append_field(Value::Array(children))
                .build()
                .expect("estrutura do dbusmenu"),
        )
    }
}

impl Serialize for LayoutItem {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut item = serializer.serialize_struct("LayoutItem", 3)?;
        item.serialize_field("id", &self.id)?;
        item.serialize_field("props", &self.props)?;
        let children: Vec<Value<'static>> = self.children.iter().map(Self::as_value).collect();
        item.serialize_field("children", &children)?;
        item.end()
    }
}

// ---------------------------------------------------------------------------
// Interface D-Bus
// ---------------------------------------------------------------------------

struct DBusMenu {
    state: Arc<Mutex<MenuState>>,
    commands: mpsc::Sender<MenuCommand>,
    /// O egui só redesenha sob demanda: sem este empurrão o comando ficaria
    /// parado no canal até a próxima interação com a janela.
    repaint: egui::Context,
}

#[interface(name = "com.canonical.dbusmenu")]
impl DBusMenu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }

    #[zbus(property)]
    fn text_direction(&self) -> String {
        "ltr".to_owned()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "normal".to_owned()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        Vec::new()
    }

    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, LayoutItem) {
        let state = self.state.lock().expect("menu");
        let depth = if recursion_depth < 0 {
            i32::MAX
        } else {
            recursion_depth
        };
        (state.revision, state.layout(parent_id, depth))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        _property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let state = self.state.lock().expect("menu");
        ids.into_iter()
            .map(|id| (id, state.properties(id)))
            .collect()
    }

    fn get_property(&self, id: i32, name: String) -> fdo::Result<OwnedValue> {
        let state = self.state.lock().expect("menu");
        state
            .properties(id)
            .remove(&name)
            .ok_or_else(|| fdo::Error::InvalidArgs(format!("propriedade {name} não existe")))
    }

    fn event(&self, id: i32, event_id: String, _data: Value<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        let command = self
            .state
            .lock()
            .ok()
            .and_then(|state| state.items.get(&id).and_then(|item| item.command));
        if let Some(command) = command {
            let _ = self.commands.send(command);
            self.repaint.request_repaint();
        }
    }

    fn event_group(&self, events: Vec<(i32, String, OwnedValue, u32)>) -> Vec<i32> {
        for (id, event_id, data, timestamp) in events {
            self.event(id, event_id, Value::from(data), timestamp);
        }
        Vec::new()
    }

    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (Vec::new(), Vec::new())
    }

    #[zbus(signal)]
    async fn items_properties_updated(
        emitter: &SignalEmitter<'_>,
        updated: Vec<(i32, HashMap<String, OwnedValue>)>,
        removed: Vec<(i32, Vec<String>)>,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn layout_updated(
        emitter: &SignalEmitter<'_>,
        revision: u32,
        parent: i32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn item_activation_requested(
        emitter: &SignalEmitter<'_>,
        id: i32,
        timestamp: u32,
    ) -> zbus::Result<()>;
}

// ---------------------------------------------------------------------------
// Handle usado pela interface gráfica
// ---------------------------------------------------------------------------

enum Request {
    /// Publica a nova árvore e avisa o painel.
    Refresh,
    /// Registra uma janela X11 no registrador de menus.
    RegisterX11(u32),
}

pub struct GlobalMenu {
    state: Arc<Mutex<MenuState>>,
    requests: tokio::sync::mpsc::UnboundedSender<Request>,
    commands: mpsc::Receiver<MenuCommand>,
    /// Nome único da conexão e caminho do objeto, para entregar ao compositor.
    pub address: (String, String),
    model: MenuModel,
}

impl GlobalMenu {
    /// Publica o serviço e devolve o handle; `None` quando não há sessão D-Bus.
    pub fn spawn(repaint: egui::Context) -> Option<Self> {
        let state = Arc::new(Mutex::new(MenuState::default()));
        let (commands_tx, commands_rx) = mpsc::channel();
        let (requests_tx, mut requests_rx) = tokio::sync::mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = mpsc::channel();

        let thread_state = Arc::clone(&state);
        std::thread::Builder::new()
            .name("papo-menu".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        log::warn!("menu global sem runtime: {error}");
                        let _ = ready_tx.send(None);
                        return;
                    }
                };

                runtime.block_on(async move {
                    let menu = DBusMenu {
                        state: Arc::clone(&thread_state),
                        commands: commands_tx,
                        repaint,
                    };
                    let connection = match zbus::connection::Builder::session()
                        .and_then(|builder| builder.serve_at(OBJECT_PATH, menu))
                        .map(|builder| builder.build())
                    {
                        Ok(future) => match future.await {
                            Ok(connection) => connection,
                            Err(error) => {
                                log::warn!("menu global não publicou: {error}");
                                let _ = ready_tx.send(None);
                                return;
                            }
                        },
                        Err(error) => {
                            log::warn!("menu global não publicou: {error}");
                            let _ = ready_tx.send(None);
                            return;
                        }
                    };

                    let name = connection
                        .unique_name()
                        .map(|name| name.to_string())
                        .unwrap_or_default();
                    log::info!("menu global em {name}{OBJECT_PATH}");
                    let _ = ready_tx.send(Some(name));

                    while let Some(request) = requests_rx.recv().await {
                        match request {
                            Request::Refresh => {
                                let revision =
                                    thread_state.lock().map(|s| s.revision).unwrap_or_default();
                                if let Ok(iface) = connection
                                    .object_server()
                                    .interface::<_, DBusMenu>(OBJECT_PATH)
                                    .await
                                {
                                    let _ = DBusMenu::layout_updated(
                                        iface.signal_emitter(),
                                        revision,
                                        ROOT_ID,
                                    )
                                    .await;
                                }
                            }
                            Request::RegisterX11(window) => {
                                register_x11_window(&connection, window).await;
                            }
                        }
                    }
                });
            })
            .ok()?;

        let name = ready_rx
            .recv_timeout(std::time::Duration::from_secs(3))
            .ok()
            .flatten()?;

        Some(Self {
            state,
            requests: requests_tx,
            commands: commands_rx,
            address: (name, OBJECT_PATH.to_owned()),
            model: MenuModel::default(),
        })
    }

    /// Troca a árvore do menu, se ela mudou.
    pub fn set_model(&mut self, model: MenuModel) {
        if self.model == model {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            state.load(&model);
        }
        self.model = model;
        let _ = self.requests.send(Request::Refresh);
    }

    /// Associa o menu a uma janela X11 (inclui XWayland).
    pub fn register_x11(&self, window: u32) {
        let _ = self.requests.send(Request::RegisterX11(window));
    }

    pub fn try_recv(&self) -> Option<MenuCommand> {
        self.commands.try_recv().ok()
    }
}

/// Registro no X11: propriedades na janela e aviso ao registrador do KDE.
async fn register_x11_window(connection: &zbus::Connection, window: u32) {
    let name = connection
        .unique_name()
        .map(|name| name.to_string())
        .unwrap_or_default();

    if let Err(error) = set_x11_appmenu_properties(window, &name, OBJECT_PATH) {
        log::warn!("propriedades de menu no X11 falharam: {error}");
    }

    let call = connection
        .call_method(
            Some("com.canonical.AppMenu.Registrar"),
            "/com/canonical/AppMenu/Registrar",
            Some("com.canonical.AppMenu.Registrar"),
            "RegisterWindow",
            &(window, zvariant::ObjectPath::try_from(OBJECT_PATH).unwrap()),
        )
        .await;
    if let Err(error) = call {
        log::warn!("registrador de menus recusou: {error}");
    }
}

fn set_x11_appmenu_properties(
    window: u32,
    service: &str,
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::{ConnectionExt as _, PropMode};
    use x11rb::wrapper::ConnectionExt as _;

    let (conn, _screen) = x11rb::connect(None)?;
    let utf8 = conn.intern_atom(false, b"UTF8_STRING")?.reply()?.atom;
    let service_atom = conn
        .intern_atom(false, b"_KDE_NET_WM_APPMENU_SERVICE_NAME")?
        .reply()?
        .atom;
    let path_atom = conn
        .intern_atom(false, b"_KDE_NET_WM_APPMENU_OBJECT_PATH")?
        .reply()?
        .atom;

    conn.change_property8(
        PropMode::REPLACE,
        window,
        service_atom,
        utf8,
        service.as_bytes(),
    )?;
    conn.change_property8(PropMode::REPLACE, window, path_atom, utf8, path.as_bytes())?;
    conn.flush()?;
    Ok(())
}
