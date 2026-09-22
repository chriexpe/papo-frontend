//! EditText Android para campos comuns, isolado do editor do chat.
//!
//! Cada chave corresponde a uma View Android própria. Não há dono global,
//! fallback nem troca de Editable entre campos.

use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock},
};

use android_activity::AndroidApp;

static APP: OnceLock<AndroidApp> = OnceLock::new();
static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> &'static Mutex<State> {
    STATE.get_or_init(|| Mutex::new(State::default()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Text,
    Password,
    Search,
}

impl Mode {
    const fn java(self) -> i32 {
        match self {
            Self::Text => 0,
            Self::Password => 1,
            Self::Search => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Spec {
    key: String,
    text: String,
    hint: String,
    left: i32,
    top: i32,
    width: i32,
    height: i32,
    text_size_px: i32,
    text_color: i32,
    hint_color: i32,
    mode: i32,
    max_chars: i32,
}

#[derive(Default)]
struct FieldState {
    incoming: Option<String>,
    submit: bool,
    focused: bool,
    last: Option<Spec>,
}

#[derive(Default)]
struct State {
    seen: HashSet<String>,
    fields: HashMap<String, FieldState>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Events {
    pub changed: bool,
    pub submit: bool,
    pub focused: bool,
}

pub fn install(app: AndroidApp) {
    let _ = APP.set(app);
}

pub fn begin_frame() {
    if let Ok(mut state) = state().lock() {
        state.seen.clear();
    }
}

pub fn end_frame() {
    let stale = if let Ok(mut state) = state().lock() {
        let stale = state
            .fields
            .keys()
            .filter(|key| !state.seen.contains(*key))
            .cloned()
            .collect::<Vec<_>>();
        for key in &stale {
            state.fields.remove(key);
        }
        stale
    } else {
        Vec::new()
    };
    for key in stale {
        call_hide(&key);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    key: &str,
    text: &mut String,
    rect: egui::Rect,
    hint: &str,
    mode: Mode,
    max_chars: usize,
    request_focus: bool,
    text_color: egui::Color32,
    hint_color: egui::Color32,
    font_size_points: f32,
) -> Events {
    let ppp = ctx.pixels_per_point().max(0.1);
    let mut changed = false;

    let spec = Spec {
        key: key.to_owned(),
        text: text.clone(),
        hint: hint.to_owned(),
        left: (rect.min.x * ppp).round() as i32,
        top: (rect.min.y * ppp).round() as i32,
        width: (rect.width() * ppp).round().max(1.0) as i32,
        height: (rect.height() * ppp).round().max(1.0) as i32,
        text_size_px: (font_size_points * ppp).round().max(1.0) as i32,
        text_color: android_color(text_color),
        hint_color: android_color(hint_color),
        mode: mode.java(),
        max_chars: max_chars.min(i32::MAX as usize) as i32,
    };

    let (submit, focused, send) = {
        let Ok(mut state) = state().lock() else {
            return Events::default();
        };
        state.seen.insert(key.to_owned());
        let field = state.fields.entry(key.to_owned()).or_default();

        if let Some(incoming) = field.incoming.take() {
            let incoming = if max_chars == 0 {
                incoming
            } else {
                incoming.chars().take(max_chars).collect()
            };
            if *text != incoming {
                *text = incoming;
                changed = true;
            }
        }

        let mut spec = spec;
        spec.text.clone_from(text);
        let send = request_focus || field.last.as_ref() != Some(&spec);
        if send {
            field.last = Some(spec.clone());
        }
        (std::mem::take(&mut field.submit), field.focused, send.then_some(spec))
    };

    if let Some(spec) = send {
        call_show(&spec, request_focus);
    }

    Events {
        changed,
        submit,
        focused,
    }
}

fn android_color(color: egui::Color32) -> i32 {
    i32::from_be_bytes([color.a(), color.r(), color.g(), color.b()])
}

fn call_show(spec: &Spec, focus: bool) {
    let Some(app) = APP.get() else { return; };
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }) else { return; };
    let Ok(mut env) = vm.attach_current_thread() else { return; };
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    let (Ok(key), Ok(text), Ok(hint)) = (
        env.new_string(&spec.key),
        env.new_string(&spec.text),
        env.new_string(&spec.hint),
    ) else {
        return;
    };
    let args = [
        jni::objects::JValue::Object(&key),
        jni::objects::JValue::Object(&text),
        jni::objects::JValue::Object(&hint),
        jni::objects::JValue::Int(spec.left),
        jni::objects::JValue::Int(spec.top),
        jni::objects::JValue::Int(spec.width),
        jni::objects::JValue::Int(spec.height),
        jni::objects::JValue::Float(spec.text_size_px as f32),
        jni::objects::JValue::Int(spec.text_color),
        jni::objects::JValue::Int(spec.hint_color),
        jni::objects::JValue::Int(spec.mode),
        jni::objects::JValue::Int(spec.max_chars),
        jni::objects::JValue::Bool(u8::from(focus)),
    ];
    if let Err(error) = env.call_method(
        &activity,
        "showNativeField",
        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;IIIIFIIIIZ)V",
        &args,
    ) {
        let _ = env.exception_clear();
        log::error!("showNativeField falhou: {error}");
    }
}

fn call_hide(key: &str) {
    let Some(app) = APP.get() else { return; };
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }) else { return; };
    let Ok(mut env) = vm.attach_current_thread() else { return; };
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    let Ok(key) = env.new_string(key) else { return; };
    let args = [jni::objects::JValue::Object(&key)];
    if let Err(error) = env.call_method(
        &activity,
        "hideNativeField",
        "(Ljava/lang/String;)V",
        &args,
    ) {
        let _ = env.exception_clear();
        log::error!("hideNativeField falhou: {error}");
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeFieldTextChanged(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    key: jni::objects::JString,
    text: jni::objects::JString,
) {
    let (Ok(key), Ok(text)) = (env.get_string(&key), env.get_string(&text)) else {
        return;
    };
    if let Ok(mut state) = state().lock()
        && let Some(field) = state.fields.get_mut(String::from(key).as_str())
    {
        field.incoming = Some(String::from(text));
    }
    super::wake::request();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeFieldSubmit(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    key: jni::objects::JString,
) {
    let Ok(key) = env.get_string(&key) else { return; };
    if let Ok(mut state) = state().lock()
        && let Some(field) = state.fields.get_mut(String::from(key).as_str())
    {
        field.submit = true;
    }
    super::wake::request();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeFieldFocusChanged(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    key: jni::objects::JString,
    focused: jni::sys::jboolean,
) {
    let Ok(key) = env.get_string(&key) else { return; };
    if let Ok(mut state) = state().lock()
        && let Some(field) = state.fields.get_mut(String::from(key).as_str())
    {
        field.focused = focused != 0;
    }
    super::wake::request();
}
