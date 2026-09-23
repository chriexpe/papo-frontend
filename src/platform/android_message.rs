//! Notificações nativas de mensagens no Android.
//!
//! Esta camada não tenta manter a rede viva nem inventa transporte em
//! background. Ela transforma notificações que o cliente já recebeu em
//! notificações nativas e recebe o alvo de navegação quando o usuário toca.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default)]
struct Snapshot {
    enabled: bool,
    active: bool,
    server_url: String,
    server_label: String,
    selected_channel: String,
    me: String,
    channels: HashMap<String, String>,
    members: HashMap<String, String>,
}

/// Metadados mínimos que a thread de rede precisa para montar uma
/// notificação legível sem esperar o egui desenhar outro quadro.
#[derive(Debug, Default)]
pub struct Context(RwLock<Snapshot>);

impl Context {
    pub fn new(server_url: String, server_label: String) -> Arc<Self> {
        Arc::new(Self(RwLock::new(Snapshot {
            server_url,
            server_label,
            ..Snapshot::default()
        })))
    }
}

pub fn sync_context(
    context: &Arc<Context>,
    enabled: bool,
    active: bool,
    server_label: &str,
    selected_channel: &str,
    me: &str,
    channels: impl IntoIterator<Item = (String, String)>,
    members: impl IntoIterator<Item = (String, String)>,
) {
    let Ok(mut snapshot) = context.0.write() else {
        return;
    };
    snapshot.enabled = enabled;
    snapshot.active = active;
    snapshot.server_label = server_label.to_owned();
    snapshot.selected_channel = selected_channel.to_owned();
    snapshot.me = me.to_owned();
    snapshot.channels = channels.into_iter().collect();
    snapshot.members = members.into_iter().collect();
}

/// Chamado diretamente pela thread de rede quando a notificação já tem
/// channel_id. A decisão de suprimir a conversa aberta usa o lifecycle
/// nativo, portanto continua correta mesmo quando o egui parou de desenhar.
pub fn received(context: &Arc<Context>, notification: &crate::api::models::Notification) {
    let Ok(snapshot) = context.0.read() else {
        return;
    };
    if !snapshot.enabled || notification.read {
        return;
    }
    let Some(channel_id) = notification.channel_id.as_deref() else {
        return;
    };
    let Some(message_id) = notification.message_id.as_deref() else {
        return;
    };
    if notification.author_id.as_deref() == Some(snapshot.me.as_str()) {
        return;
    }

    let viewing = crate::platform::android_call::is_foreground()
        && snapshot.active
        && snapshot.selected_channel == channel_id;
    if viewing {
        return;
    }

    let author = notification
        .author_id
        .as_deref()
        .and_then(|id| snapshot.members.get(id))
        .cloned()
        .or_else(|| notification.author_id.clone())
        .unwrap_or_else(|| "Papo".to_owned());
    let channel = snapshot
        .channels
        .get(channel_id)
        .map(|name| format!("#{name}"))
        .unwrap_or_default();

    let title = match (channel.is_empty(), snapshot.server_label.is_empty()) {
        (true, true) => author,
        (true, false) => format!("{author} · {}", snapshot.server_label),
        (false, true) => format!("{author} · {channel}"),
        (false, false) => format!("{author} · {channel} · {}", snapshot.server_label),
    };

    show(&NativeNotification {
        title,
        body: notification.message_content.clone().unwrap_or_default(),
        server_url: snapshot.server_url.clone(),
        channel_id: channel_id.to_owned(),
        message_id: message_id.to_owned(),
        notification_id: notification.id.clone(),
    });
}

#[derive(Debug, Clone, Serialize)]
pub struct NativeNotification {
    pub title: String,
    pub body: String,
    pub server_url: String,
    pub channel_id: String,
    pub message_id: String,
    pub notification_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Navigation {
    pub server_url: String,
    pub channel_id: String,
    pub message_id: String,
    pub notification_id: String,
}

static PENDING: Mutex<Vec<Navigation>> = Mutex::new(Vec::new());
static PERMISSION_REQUESTED: AtomicBool = AtomicBool::new(false);

pub fn ensure_permission() {
    if PERMISSION_REQUESTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let _ = super::jvm::call_activity(
        "ensureMessageNotificationPermission",
        "()V",
        None,
    );
}

pub fn show(notification: &NativeNotification) {
    let Ok(payload) = serde_json::to_string(notification) else {
        return;
    };
    let _ = super::jvm::call_activity(
        "showMessageNotification",
        "(Ljava/lang/String;)V",
        Some(&payload),
    );
}

pub fn clear_channel(server_url: &str, channel_id: &str) {
    let payload = serde_json::json!({
        "server_url": server_url,
        "channel_id": channel_id,
    })
    .to_string();
    let _ = super::jvm::call_activity(
        "clearMessageNotifications",
        "(Ljava/lang/String;)V",
        Some(&payload),
    );
}

pub fn take_navigation() -> Option<Navigation> {
    PENDING.lock().ok().and_then(|mut pending| {
        if pending.is_empty() {
            None
        } else {
            Some(pending.remove(0))
        }
    })
}

pub fn defer_navigation(target: Navigation) {
    if let Ok(mut pending) = PENDING.lock() {
        pending.insert(0, target);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeMessageNotificationTapped(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    payload: jni::objects::JString,
) {
    let Ok(payload) = env.get_string(&payload) else {
        return;
    };
    let payload: String = payload.into();
    let Ok(target) = serde_json::from_str::<Navigation>(&payload) else {
        return;
    };
    if let Ok(mut pending) = PENDING.lock() {
        pending.push(target);
    }
    super::wake::request();
}
