//! Token FCM deste aparelho, entregue pela Activity.
//!
//! O Java obtém o token (e o recebe de novo quando o Firebase o troca); o
//! registro em cada servidor é decidido pelo app, que sabe em quais há
//! sessão. Sem configuração do Firebase no APK o token nunca chega e tudo
//! continua na reconciliação periódica.

use std::sync::Mutex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub token: String,
    pub name: Option<String>,
}

static DEVICE: Mutex<Option<Device>> = Mutex::new(None);

pub fn device() -> Option<Device> {
    DEVICE.lock().ok().and_then(|device| device.clone())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativePushTokenChanged(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    token: jni::objects::JString,
    device_name: jni::objects::JString,
) {
    let Ok(token) = env.get_string(&token).map(String::from) else {
        return;
    };
    if token.is_empty() {
        return;
    }
    let name = env
        .get_string(&device_name)
        .ok()
        .map(String::from)
        .map(|name| name.trim().chars().take(32).collect::<String>())
        .filter(|name| !name.is_empty());
    if let Ok(mut device) = DEVICE.lock() {
        *device = Some(Device { token, name });
    }
    super::wake::request();
}
