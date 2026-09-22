//! EditText Android real sobre a superfície do egui.
//!
//! O egui continua dono do layout, mas o Android é dono da edição: texto,
//! cursor, seleção, composição, autocorreção, repetição e scroll. Existe uma
//! única View nativa; o compositor é o dono de reserva e campos tocados tomam
//! posse temporariamente.

use std::sync::{Mutex, OnceLock};

use android_activity::AndroidApp;

static APP: OnceLock<AndroidApp> = OnceLock::new();
static STATE: Mutex<State> = Mutex::new(State::new());

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Composer,
    Edit,
    Text,
    Password,
    Search,
}

impl Mode {
    const fn java(self) -> i32 {
        match self {
            Self::Composer => 0,
            Self::Edit => 1,
            Self::Text => 2,
            Self::Password => 3,
            Self::Search => 4,
        }
    }

    const fn secret(self) -> bool {
        matches!(self, Self::Password)
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
    max_chars: i32,
}

struct State {
    active: Option<String>,
    active_seen: bool,
    fallback: Option<Spec>,
    last: Option<Spec>,
    incoming: Option<(String, String)>,
    selection: Option<(String, usize, usize)>,
    submit: Option<String>,
    focus: Option<(String, bool)>,
}

impl State {
    const fn new() -> Self {
        Self {
            active: None,
            active_seen: false,
            fallback: None,
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
    pub visible: bool,
}

pub fn install(app: AndroidApp) {
    let _ = APP.set(app);
}

pub fn begin_frame() {
    if let Ok(mut state) = STATE.lock() {
        state.active_seen = false;
        state.fallback = None;
    }
}

/// Se um campo temporário sumiu neste quadro, devolve a View ao compositor
/// imediatamente. Sem compositor disponível, esconde a View e o teclado.
pub fn end_frame() {
    let action = if let Ok(mut state) = STATE.lock() {
        if state.active_seen {
            None
        } else if let Some(spec) = state.fallback.take() {
            state.active = Some(spec.key.clone());
            state.active_seen = true;
            if state.last.as_ref() != Some(&spec) {
                state.last = Some(spec.clone());
                Some(Some(spec))
            } else {
                None
            }
        } else if state.active.is_some() || state.last.is_some() {
            state.active = None;
            state.last = None;
            state.focus = None;
            state.selection = None;
            Some(None)
        } else {
            None
        }
    } else {
        None
    };

    match action {
        Some(Some(spec)) => call_show(&spec, false),
        Some(None) => call_hide(),
        None => {}
    }
}

/// Compositor: aparece sempre que nenhum campo temporário tomou posse.
#[allow(clippy::too_many_arguments)]
pub fn show_fallback(
    ctx: &egui::Context,
    key: &str,
    text: &mut String,
    rect: egui::Rect,
    hint: &str,
    max_lines: usize,
    text_color: egui::Color32,
    hint_color: egui::Color32,
    font_size_points: f32,
) -> Events {
    show_impl(
        ctx,
        key,
        text,
        rect,
        hint,
        Mode::Composer,
        max_lines,
        0,
        false,
        true,
        text_color,
        hint_color,
        font_size_points,
    )
}

/// Campo que já decidiu tomar posse, como a edição de uma mensagem.
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
    show_impl(
        ctx,
        key,
        text,
        rect,
        hint,
        mode,
        max_lines,
        0,
        request_focus,
        false,
        text_color,
        hint_color,
        font_size_points,
    )
}

/// Campo simples. Enquanto inativo ele é apenas desenhado pelo egui; ao ser
/// tocado, a EditText nativa assume exatamente o mesmo retângulo.
#[allow(clippy::too_many_arguments)]
pub fn singleline(
    ui: &mut egui::Ui,
    key: &str,
    text: &mut String,
    rect: egui::Rect,
    hint: &str,
    mode: Mode,
    max_chars: usize,
    request_focus: bool,
    text_color: egui::Color32,
    hint_color: egui::Color32,
    font: egui::FontId,
) -> Events {
    debug_assert!(!matches!(mode, Mode::Composer | Mode::Edit));

    let response = ui.interact(
        rect,
        egui::Id::new(("android-native-text", key)),
        egui::Sense::click(),
    );
    let claim = request_focus || response.clicked();

    let events = show_impl(
        ui.ctx(),
        key,
        text,
        rect,
        hint,
        mode,
        1,
        max_chars,
        claim,
        false,
        text_color,
        hint_color,
        font.size,
    );

    if !events.visible {
        let empty = text.is_empty();
        let shown = if empty {
            hint.to_owned()
        } else if mode.secret() {
            "•".repeat(text.chars().count())
        } else {
            text.clone()
        };
        ui.painter().text(
            rect.left_center(),
            egui::Align2::LEFT_CENTER,
            shown,
            font,
            if empty { hint_color } else { text_color },
        );
    }

    events
}

#[allow(clippy::too_many_arguments)]
fn show_impl(
    ctx: &egui::Context,
    key: &str,
    text: &mut String,
    rect: egui::Rect,
    hint: &str,
    mode: Mode,
    max_lines: usize,
    max_chars: usize,
    request_focus: bool,
    fallback: bool,
    text_color: egui::Color32,
    hint_color: egui::Color32,
    font_size_points: f32,
) -> Events {
    let ppp = ctx.pixels_per_point().max(0.1);
    let mut changed = false;
    let mut submit = false;
    let mut focused = false;
    let mut selection = None;

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
        max_lines: max_lines.max(1) as i32,
        max_chars: max_chars.min(i32::MAX as usize) as i32,
    };

    let (send, visible) = {
        let Ok(mut state) = STATE.lock() else {
            return Events::default();
        };

        if state
            .incoming
            .as_ref()
            .is_some_and(|(event_key, _)| event_key == key)
        {
            if let Some((_, incoming_text)) = state.incoming.take() {
                let incoming_text = if max_chars == 0 {
                    incoming_text
                } else {
                    incoming_text.chars().take(max_chars).collect()
                };
                if *text != incoming_text {
                    *text = incoming_text;
                    changed = true;
                }
            }
        }

        if let Some((event_key, value)) = &state.focus
            && event_key == key
        {
            focused = *value;
        }
        if let Some((event_key, start, end)) = &state.selection
            && event_key == key
        {
            selection = Some((*start, *end));
        }
        if state.submit.as_deref() == Some(key) {
            state.submit = None;
            submit = true;
        }

        if fallback {
            if state.active.is_none() {
                state.active = Some(key.to_owned());
            }
            if state.active.as_deref() != Some(key) {
                state.fallback = Some(spec);
                return Events {
                    changed,
                    submit,
                    focused,
                    caret: selection.map(|(_, end)| utf16_to_char_index(text, end)),
                    visible: false,
                };
            }
        } else if request_focus {
            state.active = Some(key.to_owned());
        }

        if state.active.as_deref() != Some(key) {
            return Events {
                changed,
                submit,
                focused,
                caret: selection.map(|(_, end)| utf16_to_char_index(text, end)),
                visible: false,
            };
        }

        state.active_seen = true;
        let mut spec = spec;
        spec.text.clone_from(text);

        let send = request_focus || state.last.as_ref() != Some(&spec);
        if send {
            state.last = Some(spec.clone());
        }
        (send.then_some(spec), true)
    };

    if let Some(spec) = send {
        call_show(&spec, request_focus);
    }

    Events {
        changed,
        submit,
        focused,
        caret: selection.map(|(_, end)| utf16_to_char_index(text, end)),
        visible,
    }
}

/// Atualiza o cursor quando a alteração nasceu no lado egui (ex.: :emoji:).
pub fn set_selection(key: &str, text: &str, char_index: usize) {
    let utf16 = char_to_utf16_index(text, char_index.min(text.chars().count()));
    let active = if let Ok(mut state) = STATE.lock() {
        state.selection = Some((key.to_owned(), utf16, utf16));
        state.active.as_deref() == Some(key)
    } else {
        false
    };
    if active {
        call_selection(key, utf16 as i32);
    }
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
        jni::objects::JValue::Int(spec.max_chars),
        jni::objects::JValue::Bool(u8::from(focus)),
    ];

    if let Err(error) = env.call_method(
        &activity,
        "showNativeEditor",
        "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;IIIIFIIIIIZ)V",
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
