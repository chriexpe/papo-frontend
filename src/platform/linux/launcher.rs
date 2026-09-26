//! Contador na barra de tarefas (com.canonical.Unity.LauncherEntry).
//!
//! É um sinal solto no barramento, sem serviço do outro lado: o Plasma (e o
//! Dash do GNOME, com a extensão) escuta e desenha o número sobre o ícone do
//! `.desktop` informado.

use std::collections::HashMap;
use std::sync::mpsc;

use zbus::zvariant::Value;

const PATH: &str = "/com/canonical/unity/launcherentry/papo";
const INTERFACE: &str = "com.canonical.Unity.LauncherEntry";
const APP_URI: &str = "application://papo.desktop";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Badge {
    pub count: u32,
    /// Sem contagem, mas com coisa nova: o ícone só pisca.
    pub urgent: bool,
}

pub struct Launcher {
    updates: mpsc::Sender<Badge>,
    last: std::cell::Cell<Option<Badge>>,
}

impl Launcher {
    pub fn spawn() -> Option<Self> {
        let (tx, rx) = mpsc::channel::<Badge>();

        std::thread::Builder::new()
            .name("papo-launcher".into())
            .spawn(move || {
                let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                let connection = match runtime.block_on(zbus::Connection::session()) {
                    Ok(connection) => connection,
                    Err(error) => {
                        log::warn!("sem contador na barra de tarefas: {error}");
                        return;
                    }
                };

                while let Ok(badge) = rx.recv() {
                    if let Err(error) = runtime.block_on(emit(&connection, badge)) {
                        log::warn!("contador recusado: {error}");
                    }
                }
            })
            .ok()?;

        Some(Self {
            updates: tx,
            last: std::cell::Cell::new(None),
        })
    }

    /// Só fala com o barramento quando o número muda.
    pub fn set(&self, badge: Badge) {
        if self.last.get() == Some(badge) {
            return;
        }
        self.last.set(Some(badge));
        let _ = self.updates.send(badge);
    }
}

async fn emit(connection: &zbus::Connection, badge: Badge) -> zbus::Result<()> {
    let mut properties: HashMap<&str, Value<'_>> = HashMap::new();
    properties.insert("count", Value::I64(badge.count as i64));
    properties.insert("count-visible", Value::Bool(badge.count > 0));
    properties.insert("urgent", Value::Bool(badge.urgent));

    connection
        .emit_signal(
            None::<&str>,
            PATH,
            INTERFACE,
            "Update",
            &(APP_URI, properties),
        )
        .await
}
