//! EditText Android real sobre a superfície do egui.
//!
//! O egui continua dono do layout e do estado da conversa, mas no Android o
//! compositor e a edição de mensagem são Views nativas. O IME passa a falar
//! diretamente com um Editable Android; daqui só sincronizamos texto, seleção
//! e a ação de concluir a edição.

use std::sync::{Mutex, OnceLock};

use android_activity::AndroidApp;

static APP: OnceLock<AndroidApp> = OnceLock::new();
static STATE: Mutex<State> = Mutex::new(State::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Composer,
    Edit,
}

impl Mode {
    const fn java(self) -> i32 {
        match self {
            Self::Composer => 0,
            Self::Edit => 1,
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
    max_lines: i32,
}

struct State {
    seen: bool,
    last: Option<Spec>,
    incoming: Option<(String, String)>,
    selection: Option<(String, usize, usize)>,
    submit: Option<String>,
    focus: Option<(String, bool)>,
}

impl State {
    const fn new() -> Self {
        Self {
            seen: false,
            last: None,
            incoming: None,
            selection: None,
            submit: None,
            focus: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Events {
    pub changed: bool,
    pub submit: bool,
    pub focused: bool,
    pub caret: Option<usize>,
}

pub fn install(app: AndroidApp) {
    let _ = APP.set(app);
}

/// Começo de um quadro. Se ninguém chamar `show`, `end_frame` remove a View.
pub fn begin_frame() {
    if let Ok(mut state) = STATE.lock() {
        state.seen = false;
    }
}

pub fn end_frame() {
    let hide = if let Ok(mut state) = STATE.lock() {
        if state.seen || state.last.is_none() {
            false
        } else {
            state.last = None;
            state.focus = None;
            state.selection = None;
            true
        }
    } else {
        false
    };
    if hide {
        call_hide();
    }
}

/// Solta imediatamente o editor nativo e seu foco.
///
/// Diferente de `end_frame`, isto é usado por interações que mudam o dono do
/// editor no meio do quadro (fechar o IME, cancelar edição, abrir gaveta).
/// O texto continua no modelo Rust; só a View Android deixa de interceptar
/// toque/foco.
pub fn dismiss() {
    let hide = if let Ok(mut state) = STATE.lock() {
        let hide = state.last.is_some();
        state.seen = false;
        state.last = None;
        state.focus = None;
        state.selection = None;
        state.submit = None;
        hide
    } else {
        false
    };
    if hide {
        call_hide();
    }
}

/// Mostra/sincroniza o editor nativo na área física correspondente ao Rect.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ctx: &egui::Context,
    key: &str,
    text: &mut String,
    rect: egui::Rect,
    hint: &str,
    mode: Mode,
    max_lines: usize,
    request_focus: bool,
    text_color: egui::Color32,
    hint_color: egui::Color32,
    font_size_points: f32,
) -> Events {
    let mut changed = false;
    let mut submit = false;
    let mut focused = false;
    let mut selection = None;

    let ppp = ctx.pixels_per_point().max(0.1);
    let spec;

    {
        let Ok(mut state) = STATE.lock() else {
            return Events::default();
        };
        state.seen = true;

        if let Some((incoming_key, incoming_text)) = state.incoming.take()
            && incoming_key == key
            && *text != incoming_text
        {
            *text = incoming_text;
            changed = true;
        }

        if let Some((focus_key, value)) = &state.focus
            && focus_key == key
        {
            focused = *value;
        }

        if let Some((selection_key, start, end)) = &state.selection
            && selection_key == key
        {
            selection = Some((*start, *end));
        }

        if state.submit.as_deref() == Some(key) {
            state.submit = None;
            submit = true;
        }

        spec = Spec {
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
            max_lines: max_lines.max(1) as i32,
        };

        let send = request_focus || state.last.as_ref() != Some(&spec);
        if send {
            state.last = Some(spec.clone());
        } else {
            return Events {
                changed,
                submit,
                focused,
                caret: selection.map(|(_, end)| utf16_to_char_index(text, end)),
            };
        }
    }

    call_show(&spec, request_focus);

    Events {
        changed,
        submit,
        focused,
        caret: selection.map(|(_, end)| utf16_to_char_index(text, end)),
    }
}

/// Atualiza o cursor quando a alteração nasceu no lado egui.
pub fn set_selection(key: &str, text: &str, char_index: usize) {
    let utf16 = char_to_utf16_index(text, char_index.min(text.chars().count()));
    if let Ok(mut state) = STATE.lock() {
        state.selection = Some((key.to_owned(), utf16, utf16));
    }
    call_selection(key, utf16 as i32);
}

fn android_color(color: egui::Color32) -> i32 {
    let [r, g, b, a] = color.to_array();
    u32::from_be_bytes([a, r, g, b]) as i32
}

fn call_show(spec: &Spec, focus: bool) {
    let Some(app) = APP.get() else {
        return;
    };
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }) else {
        return;
    };
    let Ok(mut env) = vm.attach_current_thread() else {
        return;
    };
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
        jni::objects::JValue::Int(spec.max_lines),
        jni::objects::JValue::Bool(u8::from(focus)),
    ];

    if let Err(error) = env.call_method(
        &activity,
        "showNativeEditor",
        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;IIIIFIIIIZ)V",
        &args,
    ) {
        let _ = env.exception_clear();
        log::error!("showNativeEditor falhou: {error}");
    }
}

fn call_selection(key: &str, selection: i32) {
    let Some(app) = APP.get() else {
        return;
    };
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }) else {
        return;
    };
    let Ok(mut env) = vm.attach_current_thread() else {
        return;
    };
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    let Ok(key) = env.new_string(key) else {
        return;
    };
    let args = [
        jni::objects::JValue::Object(&key),
        jni::objects::JValue::Int(selection),
    ];
    if let Err(error) = env.call_method(
        &activity,
        "setNativeEditorSelection",
        "(Ljava/lang/String;I)V",
        &args,
    ) {
        let _ = env.exception_clear();
        log::error!("setNativeEditorSelection falhou: {error}");
    }
}

fn call_hide() {
    let Some(app) = APP.get() else {
        return;
    };
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(app.vm_as_ptr().cast()) }) else {
        return;
    };
    let Ok(mut env) = vm.attach_current_thread() else {
        return;
    };
    let activity = unsafe { jni::objects::JObject::from_raw(app.activity_as_ptr().cast()) };
    if let Err(error) = env.call_method(&activity, "hideNativeEditor", "()V", &[]) {
        let _ = env.exception_clear();
        log::error!("hideNativeEditor falhou: {error}");
    }
}

fn char_to_utf16_index(text: &str, char_index: usize) -> usize {
    text.chars().take(char_index).map(char::len_utf16).sum()
}

fn utf16_to_char_index(text: &str, utf16_index: usize) -> usize {
    let mut units = 0;
    let mut chars = 0;
    for ch in text.chars() {
        let next = units + ch.len_utf16();
        if next > utf16_index {
            break;
        }
        units = next;
        chars += 1;
    }
    chars
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeEditorTextChanged(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    key: jni::objects::JString,
    text: jni::objects::JString,
) {
    let (Ok(key), Ok(text)) = (env.get_string(&key), env.get_string(&text)) else {
        return;
    };
    if let Ok(mut state) = STATE.lock() {
        state.incoming = Some((String::from(key), String::from(text)));
    }
    super::wake::request();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeEditorSelectionChanged(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    key: jni::objects::JString,
    start: jni::sys::jint,
    end: jni::sys::jint,
) {
    let Ok(key) = env.get_string(&key) else {
        return;
    };
    if let Ok(mut state) = STATE.lock() {
        state.selection = Some((
            String::from(key),
            start.max(0) as usize,
            end.max(0) as usize,
        ));
    }
    super::wake::request();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeEditorSubmit(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    key: jni::objects::JString,
) {
    let Ok(key) = env.get_string(&key) else {
        return;
    };
    if let Ok(mut state) = STATE.lock() {
        state.submit = Some(String::from(key));
    }
    super::wake::request();
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeEditorFocusChanged(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    key: jni::objects::JString,
    focused: jni::sys::jboolean,
) {
    let Ok(key) = env.get_string(&key) else {
        return;
    };
    if let Ok(mut state) = STATE.lock() {
        state.focus = Some((String::from(key), focused != 0));
    }
    super::wake::request();
}

#[cfg(test)]
mod tests {
    use super::{char_to_utf16_index, utf16_to_char_index};

    #[test]
    fn indices_utf16_preservam_emoji() {
        let text = "a😀b";
        assert_eq!(char_to_utf16_index(text, 2), 3);
        assert_eq!(utf16_to_char_index(text, 3), 2);
    }
}
