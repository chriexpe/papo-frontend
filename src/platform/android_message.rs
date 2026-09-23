//! Notificações nativas de mensagens no Android.
//!
//! Esta camada não tenta manter a rede viva nem inventa transporte em
//! background. Ela transforma notificações que o cliente já recebeu em
//! notificações nativas e recebe o alvo de navegação quando o usuário toca.

use std::collections::{HashMap, VecDeque};
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
    my_username: String,
    channels: HashMap<String, String>,
    members: HashMap<String, String>,
    usernames: HashMap<String, String>,
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
    my_username: &str,
    channels: impl IntoIterator<Item = (String, String)>,
    members: impl IntoIterator<Item = (String, String)>,
    usernames: impl IntoIterator<Item = (String, String)>,
) {
    let Ok(mut snapshot) = context.0.write() else {
        return;
    };
    snapshot.enabled = enabled;
    snapshot.active = active;
    snapshot.server_label = server_label.to_owned();
    snapshot.selected_channel = selected_channel.to_owned();
    snapshot.me = me.to_owned();
    snapshot.my_username = my_username.to_owned();
    snapshot.channels = channels.into_iter().collect();
    snapshot.members = members.into_iter().collect();
    snapshot.usernames = usernames.into_iter().collect();
}

static DELIVERED: Mutex<VecDeque<String>> = Mutex::new(VecDeque::new());

fn delivered_once(server_url: &str, message_id: &str) -> bool {
    let key = format!("{server_url}\n{message_id}");
    let Ok(mut delivered) = DELIVERED.lock() else {
        return false;
    };
    if delivered.iter().any(|known| known == &key) {
        return false;
    }
    delivered.push_back(key);
    while delivered.len() > 256 {
        delivered.pop_front();
    }
    true
}

fn display_mentions(snapshot: &Snapshot, text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '<' && i + 3 < chars.len() && chars[i + 1] == '@' {
            let mut end = i + 2;
            while end < chars.len() && chars[end] != '>' {
                end += 1;
            }
            if end < chars.len() {
                let id: String = chars[i + 2..end].iter().collect();
                if let Some(username) = snapshot.usernames.get(&id) {
                    out.push('@');
                    out.push_str(username);
                    i = end + 1;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn post(
    snapshot: &Snapshot,
    channel_id: &str,
    message_id: &str,
    author_id: Option<&str>,
    body: &str,
    notification_id: &str,
) {
    if !snapshot.enabled || author_id == Some(snapshot.me.as_str()) {
        return;
    }

    let viewing = crate::platform::android_call::is_foreground()
        && snapshot.active
        && snapshot.selected_channel == channel_id;
    if viewing || !delivered_once(&snapshot.server_url, message_id) {
        return;
    }

    let author = author_id
        .and_then(|id| snapshot.members.get(id))
        .cloned()
        .or_else(|| author_id.map(str::to_owned))
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
        body: display_mentions(snapshot, body),
        server_url: snapshot.server_url.clone(),
        channel_id: channel_id.to_owned(),
        message_id: message_id.to_owned(),
        notification_id: notification_id.to_owned(),
    });
}

/// Toda mensagem legível chega pelo socket, independentemente de
/// new_notification. Para menções usamos a mesma sintaxe que o próprio Papo
/// destaca na timeline: @username, @everyone e @todos.
pub fn received_message(context: &Arc<Context>, message: &crate::api::models::Message) {
    let Ok(snapshot) = context.0.read() else {
        return;
    };
    if !snapshot.enabled || message.author_id == snapshot.me {
        return;
    }

    let content = message.content.as_deref().unwrap_or("");
    let lower = content.to_lowercase();
    let mentioned = (!snapshot.me.is_empty()
        && lower.contains(&format!("<@{}>", snapshot.me.to_lowercase())))
        || (!snapshot.my_username.is_empty()
            && lower.contains(&format!("@{}", snapshot.my_username.to_lowercase())))
        || lower.contains("@everyone")
        || lower.contains("@todos");
    if !mentioned {
        return;
    }

    post(
        &snapshot,
        &message.channel_id,
        &message.id,
        Some(&message.author_id),
        content,
        &message.id,
    );
}

/// new_notification continua útil para replies e canais em modo all.
/// Menções que já foram publicadas pelo evento message são deduplicadas por
/// message_id.
pub fn received(context: &Arc<Context>, notification: &crate::api::models::Notification) {
    let Ok(snapshot) = context.0.read() else {
        return;
    };
    if notification.read {
        return;
    }
    let (Some(channel_id), Some(message_id)) = (
        notification.channel_id.as_deref(),
        notification.message_id.as_deref(),
    ) else {
        return;
    };

    post(
        &snapshot,
        channel_id,
        message_id,
        notification.author_id.as_deref(),
        notification.message_content.as_deref().unwrap_or(""),
        &notification.id,
    );
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
