//! Notificações nativas de mensagens no Android.
//!
//! Esta camada não tenta manter a rede viva nem inventa transporte em
//! background. Ela transforma notificações que o cliente já recebeu em
//! notificações nativas e recebe o alvo de navegação quando o usuário toca.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

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
