//! Android implementation of the WebEmbed browser surface.
//!
//! Rust owns policy and geometry. `PapoActivity` owns the actual WebView and
//! reports only narrow lifecycle/navigation/gesture events back through JNI.

use std::{
    collections::VecDeque,
    sync::{Mutex, OnceLock},
};

use android_activity::AndroidApp;

use crate::webembed::{EmbedViewport, WebEmbedBackend, WebEmbedEvent};

static APP: OnceLock<AndroidApp> = OnceLock::new();
static EVENTS: OnceLock<Mutex<VecDeque<WebEmbedEvent>>> = OnceLock::new();

fn events() -> &'static Mutex<VecDeque<WebEmbedEvent>> {
    EVENTS.get_or_init(|| Mutex::new(VecDeque::new()))
}

pub fn install(app: AndroidApp) {
    let _ = APP.set(app);
}

pub struct AndroidWebEmbedBackend;

impl WebEmbedBackend for AndroidWebEmbedBackend {
    fn create(&mut self, id: &str, url: &str) -> Result<(), String> {
        call_strings("createWebEmbed", id, Some(url))
    }

    fn present(&mut self, id: &str, viewport: &EmbedViewport) {
        call_present(id, viewport);
    }

    fn suspend(&mut self, id: &str) {
        if let Err(error) = call_strings("suspendWebEmbed", id, None) {
            log::warn!("webembed suspend falhou: {error}");
        }
    }

    fn resume(&mut self, id: &str) {
        if let Err(error) = call_strings("resumeWebEmbed", id, None) {
            log::warn!("webembed resume falhou: {error}");
        }
    }

    fn destroy(&mut self, id: &str) {
        if let Err(error) = call_strings("destroyWebEmbed", id, None) {
            log::warn!("webembed destroy falhou: {error}");
        }
    }

    fn poll_events(&mut self) -> Vec<WebEmbedEvent> {
        let Ok(mut events) = events().lock() else {
            return Vec::new();
        };
        events.drain(..).collect()
    }
}

fn with_env<T>(
    f: impl FnOnce(&mut jni::JNIEnv<'_>, jni::objects::JObject<'_>) -> Result<T, String>,
) -> Result<T, String> {
    let app = APP.get().ok_or_else(|| "Activity não instalada".to_owned())?;
    let vm = unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }
        .map_err(|error| error.to_string())?;
    let mut env = vm.attach_current_thread().map_err(|error| error.to_string())?;
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    let result = f(&mut env, activity);
    if result.is_err() {
        let _ = env.exception_clear();
    }
    result
}

fn call_strings(method: &str, id: &str, second: Option<&str>) -> Result<(), String> {
    with_env(|env, activity| {
        let id = env.new_string(id).map_err(|error| error.to_string())?;
        if let Some(second) = second {
            let second = env.new_string(second).map_err(|error| error.to_string())?;
            env.call_method(
                &activity,
                method,
                "(Ljava/lang/String;Ljava/lang/String;)V",
                &[
                    jni::objects::JValue::Object(&id),
                    jni::objects::JValue::Object(&second),
                ],
            )
            .map_err(|error| error.to_string())?;
        } else {
            env.call_method(
                &activity,
                method,
                "(Ljava/lang/String;)V",
                &[jni::objects::JValue::Object(&id)],
            )
            .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
}

fn call_present(id: &str, viewport: &EmbedViewport) {
    let ppp = viewport.pixels_per_point.max(0.1);
    let visible = viewport.rect.intersect(viewport.clip_rect);

    let left = (viewport.rect.min.x * ppp).round() as i32;
    let top = (viewport.rect.min.y * ppp).round() as i32;
    let width = (viewport.rect.width() * ppp).round().max(1.0) as i32;
    let height = (viewport.rect.height() * ppp).round().max(1.0) as i32;
    let clip_left = (visible.min.x * ppp).round() as i32;
    let clip_top = (visible.min.y * ppp).round() as i32;
    let clip_width = (visible.width() * ppp).round().max(1.0) as i32;
    let clip_height = (visible.height() * ppp).round().max(1.0) as i32;

    let result = with_env(|env, activity| {
        let id = env.new_string(id).map_err(|error| error.to_string())?;
        env.call_method(
            &activity,
            "presentWebEmbed",
            "(Ljava/lang/String;IIIIIIII)V",
            &[
                jni::objects::JValue::Object(&id),
                jni::objects::JValue::Int(left),
                jni::objects::JValue::Int(top),
                jni::objects::JValue::Int(width),
                jni::objects::JValue::Int(height),
                jni::objects::JValue::Int(clip_left),
                jni::objects::JValue::Int(clip_top),
                jni::objects::JValue::Int(clip_width),
                jni::objects::JValue::Int(clip_height),
            ],
        )
        .map_err(|error| error.to_string())?;
        Ok(())
    });
    if let Err(error) = result {
        log::warn!("webembed present falhou: {error}");
    }
}

fn push(event: WebEmbedEvent) {
    if let Ok(mut queue) = events().lock() {
        queue.push_back(event);
    }
    crate::platform::wake::request();
}

fn string(env: &mut jni::JNIEnv<'_>, value: jni::objects::JString<'_>) -> Option<String> {
    env.get_string(&value).ok().map(Into::into)
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeWebEmbedExternal(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    id: jni::objects::JString,
    url: jni::objects::JString,
) {
    let (Some(id), Some(url)) = (string(&mut env, id), string(&mut env, url)) else {
        return;
    };
    push(WebEmbedEvent::OpenExternal { id, url });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeWebEmbedFailed(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    id: jni::objects::JString,
) {
    let Some(id) = string(&mut env, id) else {
        return;
    };
    push(WebEmbedEvent::Failed { id });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeWebEmbedScroll(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    id: jni::objects::JString,
    delta_y_px: jni::sys::jfloat,
) {
    let Some(id) = string(&mut env, id) else {
        return;
    };
    push(WebEmbedEvent::ScrollTimeline {
        id,
        delta_y_px,
    });
}
