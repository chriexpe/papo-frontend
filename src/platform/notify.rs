//! Notificações da área de trabalho (org.freedesktop.Notifications).
//!
//! Uma thread própria mantém a conexão com o barramento; a interface só
//! deposita pedidos num canal e segue desenhando.

use std::sync::mpsc;

#[derive(Clone, Debug)]
pub struct Notification {
    pub summary: String,
    pub body: String,
    /// Identificador para substituir uma notificação anterior do mesmo canal.
    pub tag: Option<String>,
}

pub struct Notifier {
    requests: mpsc::Sender<Notification>,
}

impl Notifier {
    pub fn spawn() -> Option<Self> {
        let (tx, rx) = mpsc::channel::<Notification>();

        std::thread::Builder::new()
            .name("papo-notify".into())
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
                        log::warn!("sem notificações: {error}");
                        return;
                    }
                };

                // Cada canal reaproveita o id da notificação anterior, então
                // conversas movimentadas não empilham avisos.
                let mut replaces: std::collections::HashMap<String, u32> =
                    std::collections::HashMap::new();

                while let Ok(notification) = rx.recv() {
                    let previous = notification
                        .tag
                        .as_ref()
                        .and_then(|tag| replaces.get(tag).copied())
                        .unwrap_or(0);

                    let result = runtime.block_on(show(&connection, &notification, previous));
                    match result {
                        Ok(id) => {
                            if let Some(tag) = notification.tag {
                                replaces.insert(tag, id);
                            }
                        }
                        Err(error) => log::warn!("notificação recusada: {error}"),
                    }
                }
            })
            .ok()?;

        Some(Self { requests: tx })
    }

    pub fn show(&self, notification: Notification) {
        let _ = self.requests.send(notification);
    }
}

/// Ícone embutido, entregue como pixels no hint `image-data`: assim a
/// notificação mostra a marca mesmo sem o ícone instalado no tema.
fn image_data() -> Option<zvariant::Value<'static>> {
    static ICON: std::sync::OnceLock<Option<(i32, i32, Vec<u8>)>> = std::sync::OnceLock::new();
    let (width, height, pixels) = ICON
        .get_or_init(|| {
            let image = image::load_from_memory(include_bytes!("../../assets/icon.png")).ok()?;
            let image = image
                .resize_exact(64, 64, image::imageops::FilterType::Lanczos3)
                .to_rgba8();
            Some((image.width() as i32, image.height() as i32, image.into_raw()))
        })
        .clone()?;

    // (largura, altura, bytes por linha, tem alfa, bits por amostra, canais, pixels)
    Some(zvariant::Value::from(
        zvariant::StructureBuilder::new()
            .append_field(zvariant::Value::from(width))
            .append_field(zvariant::Value::from(height))
            .append_field(zvariant::Value::from(width * 4))
            .append_field(zvariant::Value::from(true))
            .append_field(zvariant::Value::from(8i32))
            .append_field(zvariant::Value::from(4i32))
            .append_field(zvariant::Value::from(pixels))
            .build()
            .ok()?,
    ))
}

async fn show(
    connection: &zbus::Connection,
    notification: &Notification,
    replaces: u32,
) -> zbus::Result<u32> {
    let mut hints: std::collections::HashMap<&str, zvariant::Value<'_>> = [
        ("desktop-entry", zvariant::Value::from("papo")),
        // "im.received" faz o Plasma tratar como mensagem recebida.
        ("category", zvariant::Value::from("im.received")),
    ]
    .into_iter()
    .collect();
    if let Some(image) = image_data() {
        hints.insert("image-data", image);
    }

    let reply = connection
        .call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "Papo",
                replaces,
                "papo",
                notification.summary.as_str(),
                notification.body.as_str(),
                Vec::<&str>::new(),
                hints,
                -1i32,
            ),
        )
        .await?;
    reply.body().deserialize::<u32>()
}
