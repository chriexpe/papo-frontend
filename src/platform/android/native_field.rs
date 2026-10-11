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
    /// Várias linhas (a bio do cartão de perfil): Enter quebra a linha.
    Multiline,
}

impl Mode {
    const fn java(self) -> i32 {
        match self {
            Self::Text => 0,
            Self::Password => 1,
            Self::Search => 2,
            Self::Multiline => 3,
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
    /// Linhas e altura de cada linha (px) que o próprio EditText mediu, só nos
    /// campos de várias linhas. A caixa segue isso, não a conta do egui.
    lines: Option<(usize, i32)>,
}

#[derive(Default)]
struct State {
    seen: HashSet<String>,
    fields: HashMap<String, FieldState>,
    pixels_per_point: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Events {
    pub changed: bool,
    pub submit: bool,
    pub focused: bool,
}

/// Linhas que o texto ocupa no campo nativo `key` e a altura de cada uma, em
/// pontos do egui, se ele já mediu o texto de agora.
pub fn measured(key: &str) -> Option<(usize, f32)> {
    let state = state().lock().ok()?;
    let (lines, line_px) = state.fields.get(key)?.lines?;
    (lines > 0 && line_px > 0 && state.pixels_per_point > 0.0)
        .then(|| (lines, line_px as f32 / state.pixels_per_point))
}

pub fn install(app: AndroidApp) {
    install_font(&app);
    let _ = APP.set(app);
}

/// Deixa a Inter, a mesma fonte do egui, num arquivo que o Java lê
/// (`PapoActivity.papoTypeface`): os campos nativos usavam a fonte do sistema
/// e por isso o texto saía com outro tamanho e outra cara que o do egui.
fn install_font(app: &AndroidApp) {
    const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter-Regular.ttf");
    let Some(dir) = app.internal_data_path().map(|path| path.join("fonts")) else {
        return;
    };
    let path = dir.join("Inter-Regular.ttf");
    let current = std::fs::metadata(&path).map(|meta| meta.len()).ok();
    if current != Some(INTER.len() as u64)
        && let Err(error) = std::fs::create_dir_all(&dir).and_then(|()| std::fs::write(&path, INTER))
    {
        log::warn!("não consegui gravar a fonte dos campos nativos: {error}");
    }
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
        state.pixels_per_point = ppp;
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

/// ARGB "reto" para o Android. `Color32` guarda os canais já multiplicados
/// pelo alfa; passá-los assim faz uma cor translúcida (a dica, o texto
/// terciário) chegar ao EditText bem mais escura do que no egui.
fn android_color(color: egui::Color32) -> i32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    u32::from_be_bytes([a, r, g, b]) as i32
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

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeFieldLines(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    key: jni::objects::JString,
    lines: jni::sys::jint,
    line_height_px: jni::sys::jint,
) {
    let Ok(key) = env.get_string(&key) else { return; };
    if let Ok(mut state) = state().lock()
        && let Some(field) = state.fields.get_mut(String::from(key).as_str())
    {
        field.lines = Some((lines.max(0) as usize, line_height_px.max(0)));
    }
    super::wake::request();
}
