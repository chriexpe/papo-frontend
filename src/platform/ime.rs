//! Ponte entre egui TextEdit e o GameTextInput do Android.
//!
//! winit 0.30 ainda ignora InputEvent::TextEvent no backend Android, entao o
//! estado do InputConnection nao chega ao egui sozinho. A ponte abaixo trata
//! o GameTextInput como um editor de verdade: texto completo, selecao/cursor e
//! regiao de composicao.
//!
//! Importante: o texto vindo do Android e injetado como entrada do TextEdit
//! ANTES do widget rodar. Nao mutamos o String depois que o TextEdit terminou,
//! porque isso deixa o estado interno de selecao/undo do egui dessincronizado.

#[cfg(target_os = "android")]
use std::sync::{Mutex, OnceLock};

#[cfg(target_os = "android")]
use android_activity::{
    AndroidApp,
    input::{ImeOptions, InputType, TextInputAction, TextInputState, TextSpan},
};

#[cfg(target_os = "android")]
static APP: OnceLock<AndroidApp> = OnceLock::new();

#[cfg(target_os = "android")]
static BRIDGE: Mutex<Bridge> = Mutex::new(Bridge::new());

/// Tipo de campo que o Android deve anunciar ao teclado.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Multiline,
    Search,
    Password,
}

#[cfg(target_os = "android")]
#[derive(Clone, Debug, PartialEq, Eq)]
struct ImeState {
    text: String,
    // Offsets da API Android sao unidades UTF-16.
    selection: (usize, usize),
    compose: Option<(usize, usize)>,
}

#[cfg(target_os = "android")]
struct Bridge {
    /// TextEdit que atualmente e dono do documento do teclado.
    field: Option<egui::Id>,
    /// Estado mais recente vindo da Activity, ainda nao convertido em eventos.
    incoming: Option<ImeState>,
    /// Selecao Android para aplicar depois que o TextEdit consumir os eventos.
    pending_selection: Option<(usize, usize)>,
    /// Ultimo estado em que Android e egui concordavam.
    shadow: Option<ImeState>,
    /// Impede que o mesmo quadro devolva ao Android o texto antigo antes do
    /// TextEdit terminar de consumir a substituicao que acabamos de injetar.
    suppress_outbound_once: bool,
    /// O callback do IME realmente mudou o texto neste quadro.
    incoming_text_changed: bool,
}

#[cfg(target_os = "android")]
impl Bridge {
    const fn new() -> Self {
        Self {
            field: None,
            incoming: None,
            pending_selection: None,
            shadow: None,
            suppress_outbound_once: false,
            incoming_text_changed: false,
        }
    }

    fn clear(&mut self) {
        self.field = None;
        self.incoming = None;
        self.pending_selection = None;
        self.shadow = None;
        self.suppress_outbound_once = false;
        self.incoming_text_changed = false;
    }
}

#[cfg(target_os = "android")]
pub fn install(app: AndroidApp) {
    let _ = APP.set(app);
}

/// Converte o State completo do GameTextInput em eventos que o TextEdit sabe
/// processar. Executa no raw_input_hook, portanto antes da interface do quadro.
///
/// A substituicao inteira (Ctrl+A + Text) e proposital: autocorrecao, colar,
/// composicao e edicao no meio deixam de depender de adivinhar um diff.
#[cfg(target_os = "android")]
pub fn pump(ctx: &egui::Context, raw_input: &mut egui::RawInput) {
    let focused = ctx.memory(|memory| memory.focused());

    let (incoming, replace_text) = {
        let Ok(mut bridge) = BRIDGE.lock() else {
            return;
        };

        if bridge.field.is_some() && bridge.field != focused {
            bridge.clear();
            return;
        }

        let Some(incoming) = bridge.incoming.take() else {
            return;
        };
        let Some(field) = bridge.field else {
            // Ainda nao sabemos qual TextEdit recebeu o foco. sync_text_edit
            // vai semear o InputConnection assim que o widget aparecer.
            return;
        };
        if focused != Some(field) {
            return;
        }

        let replace_text = bridge
            .shadow
            .as_ref()
            .is_none_or(|shadow| shadow.text != incoming.text);

        bridge.pending_selection = Some(incoming.selection);
        bridge.incoming_text_changed = replace_text;
        bridge.shadow = Some(incoming.clone());
        bridge.suppress_outbound_once = true;

        (incoming, replace_text)
    };

    if replace_text {
        let command = egui::Modifiers::COMMAND;
        for pressed in [true, false] {
            raw_input.events.push(egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: command,
            });
        }

        if incoming.text.is_empty() {
            for pressed in [true, false] {
                raw_input.events.push(egui::Event::Key {
                    key: egui::Key::Backspace,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
        } else {
            raw_input.events.push(egui::Event::Text(incoming.text));
        }
    }
}

#[cfg(not(target_os = "android"))]
pub fn pump(_ctx: &egui::Context, _raw_input: &mut egui::RawInput) {}

/// Sincroniza o lado egui DEPOIS que um TextEdit foi desenhado.
///
/// - no primeiro foco, semeia o InputConnection com o texto e cursor reais;
/// - aplica a selecao/cursor que o IME mandou no quadro;
/// - se o usuario moveu o cursor tocando no TextEdit, devolve a selecao ao
///   Android para a proxima tecla entrar exatamente naquele ponto.
pub fn sync_text_edit(
    ctx: &egui::Context,
    id: egui::Id,
    text: &mut String,
    response_has_focus: bool,
    kind: Kind,
) -> bool {
    #[cfg(not(target_os = "android"))]
    {
        let _ = (ctx, id, text, response_has_focus, kind);
        false
    }

    #[cfg(target_os = "android")]
    {
        let focused = response_has_focus || ctx.memory(|memory| memory.focused()) == Some(id);
        if !focused {
            return false;
        }

        let Ok(mut bridge) = BRIDGE.lock() else {
            return false;
        };

        if bridge.field != Some(id) {
            bridge.field = Some(id);
            bridge.incoming = None;
            bridge.pending_selection = None;
            bridge.suppress_outbound_once = false;
            bridge.incoming_text_changed = false;

            let state = state_from_egui(ctx, id, text, None);
            bridge.shadow = Some(state.clone());
            drop(bridge);

            configure_android(kind);
            send_to_android(&state);
            return false;
        }

        if let Some(selection) = bridge.pending_selection.take() {
            set_egui_selection(ctx, id, text, selection);
        }

        let changed_from_ime = std::mem::take(&mut bridge.incoming_text_changed);

        if bridge.suppress_outbound_once {
            bridge.suppress_outbound_once = false;
            return changed_from_ime;
        }

        let compose = bridge
            .shadow
            .as_ref()
            .filter(|shadow| shadow.text == *text)
            .and_then(|shadow| shadow.compose);
        let current = state_from_egui(ctx, id, text, compose);

        let outbound = if bridge.shadow.as_ref() != Some(&current) {
            bridge.shadow = Some(current.clone());
            Some(current)
        } else {
            None
        };
        drop(bridge);

        if let Some(state) = outbound {
            send_to_android(&state);
        }

        changed_from_ime
    }
}

/// Estado completo vindo de GameTextInput.Listener.stateChanged.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeSetText(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    text: jni::objects::JString,
    selection_start: jni::sys::jint,
    selection_end: jni::sys::jint,
    composing_start: jni::sys::jint,
    composing_end: jni::sys::jint,
) {
    let Ok(text) = env.get_string(&text) else {
        return;
    };
    let text: String = text.into();
    let max = text.encode_utf16().count();
    let clamp = |value: jni::sys::jint| (value.max(0) as usize).min(max);

    let compose = if composing_start < 0 || composing_end < 0 {
        None
    } else {
        Some((clamp(composing_start), clamp(composing_end)))
    };

    if let Ok(mut bridge) = BRIDGE.lock() {
        bridge.incoming = Some(ImeState {
            text,
            selection: (clamp(selection_start), clamp(selection_end)),
            compose,
        });
    }

    super::wake::request();
}

#[cfg(target_os = "android")]
fn configure_android(kind: Kind) {
    let Some(app) = APP.get() else {
        return;
    };

    let (input_type, action) = match kind {
        Kind::Multiline => (
            InputType::TYPE_CLASS_TEXT
                | InputType::TYPE_TEXT_VARIATION_SHORT_MESSAGE
                | InputType::TYPE_TEXT_FLAG_MULTI_LINE
                | InputType::TYPE_TEXT_FLAG_CAP_SENTENCES
                | InputType::TYPE_TEXT_FLAG_AUTO_CORRECT,
            TextInputAction::None,
        ),
        Kind::Search => (
            InputType::TYPE_CLASS_TEXT | InputType::TYPE_TEXT_VARIATION_FILTER,
            TextInputAction::Search,
        ),
        Kind::Password => (
            InputType::TYPE_CLASS_TEXT
                | InputType::TYPE_TEXT_VARIATION_PASSWORD
                | InputType::TYPE_TEXT_FLAG_NO_SUGGESTIONS,
            TextInputAction::Done,
        ),
        Kind::Text => (
            InputType::TYPE_CLASS_TEXT | InputType::TYPE_TEXT_FLAG_AUTO_CORRECT,
            TextInputAction::Done,
        ),
    };

    app.set_ime_editor_info(
        input_type,
        action,
        ImeOptions::IME_FLAG_NO_FULLSCREEN,
    );
}

#[cfg(target_os = "android")]
fn state_from_egui(
    ctx: &egui::Context,
    id: egui::Id,
    text: &str,
    compose: Option<(usize, usize)>,
) -> ImeState {
    let chars = text.chars().count();
    let (start, end) = egui::TextEdit::load_state(ctx, id)
        .and_then(|state| state.cursor.char_range())
        .map(|range| {
            (
                range.secondary.index.0.min(chars),
                range.primary.index.0.min(chars),
            )
        })
        .unwrap_or((chars, chars));

    ImeState {
        text: text.to_owned(),
        selection: (
            char_to_utf16_index(text, start),
            char_to_utf16_index(text, end),
        ),
        compose,
    }
}

#[cfg(target_os = "android")]
fn set_egui_selection(
    ctx: &egui::Context,
    id: egui::Id,
    text: &str,
    selection: (usize, usize),
) {
    let start = utf16_to_char_index(text, selection.0);
    let end = utf16_to_char_index(text, selection.1);

    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange {
                primary: egui::text::CCursor::new(end),
                secondary: egui::text::CCursor::new(start),
                h_pos: None,
            }));
        state.store(ctx, id);
    }
}

#[cfg(target_os = "android")]
fn send_to_android(state: &ImeState) {
    let Some(app) = APP.get() else {
        return;
    };

    app.set_text_input_state(TextInputState {
        text: state.text.clone(),
        selection: TextSpan {
            start: state.selection.0,
            end: state.selection.1,
        },
        compose_region: state
            .compose
            .map(|(start, end)| TextSpan { start, end }),
    });
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

#[cfg(test)]
mod tests {
    use super::{char_to_utf16_index, utf16_to_char_index};

    #[test]
    fn ascii_tem_os_mesmos_indices() {
        assert_eq!(char_to_utf16_index("papo", 3), 3);
        assert_eq!(utf16_to_char_index("papo", 3), 3);
    }

    #[test]
    fn emoji_ocupa_duas_unidades_utf16() {
        let text = "a😀b";
        assert_eq!(char_to_utf16_index(text, 0), 0);
        assert_eq!(char_to_utf16_index(text, 1), 1);
        assert_eq!(char_to_utf16_index(text, 2), 3);
        assert_eq!(char_to_utf16_index(text, 3), 4);
        assert_eq!(utf16_to_char_index(text, 3), 2);
        assert_eq!(utf16_to_char_index(text, 4), 3);
    }

    #[test]
    fn offset_no_meio_de_surrogate_prende_antes_do_emoji() {
        assert_eq!(utf16_to_char_index("a😀b", 2), 1);
    }
}
