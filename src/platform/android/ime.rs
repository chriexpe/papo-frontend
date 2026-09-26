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
    /// Estado completo mais recente vindo do InputConnection.
    incoming: Option<ImeState>,
    /// Ultimo estado em que Android e egui concordavam.
    shadow: Option<ImeState>,
}

#[cfg(target_os = "android")]
impl Bridge {
    const fn new() -> Self {
        Self {
            field: None,
            incoming: None,
            shadow: None,
        }
    }

    fn clear(&mut self) {
        self.field = None;
        self.incoming = None;
        self.shadow = None;
    }
}

#[cfg(target_os = "android")]
pub fn install(app: AndroidApp) {
    let _ = APP.set(app);
}

/// Mantem o dono do documento alinhado com o foco do egui.
///
/// O winit ainda descarta o TextEvent do GameTextInput no Android. O callback
/// Java acorda o loop quando o InputConnection muda, e o estado completo e
/// aplicado por `prepare_text_edit` antes de o widget editar o buffer.
#[cfg(target_os = "android")]
pub fn pump(ctx: &egui::Context, raw_input: &mut egui::RawInput) {
    let _ = raw_input;
    let focused = ctx.memory(|memory| memory.focused());

    let Ok(mut bridge) = BRIDGE.lock() else {
        return;
    };
    // Durante a animação/reflow do teclado o egui pode passar um quadro
    // transitório sem foco. Não trate isso como troca de editor: limpar aqui
    // fazia o campo ser re-semeado no quadro seguinte e alimentava um ciclo
    // de abrir/fechar o IME. Só há troca real quando outro widget tem foco.
    if focused.is_some() && bridge.field.is_some() && bridge.field != focused {
        bridge.clear();
    }
}

#[cfg(not(target_os = "android"))]
pub fn pump(_ctx: &egui::Context, _raw_input: &mut egui::RawInput) {}

/// Aplica o documento completo do Android ANTES de o TextEdit rodar.
///
/// GameTextInput e um editor de verdade: ele guarda texto, selecao/cursor e
/// composicao. Transformar esse estado em Ctrl+A + Event::Text perde justamente
/// as semanticas de editor que Gboard/SwiftKey usam para autocorrecao, arrastar
/// o cursor pela barra de espaco e apagar selecoes/palavras. Aqui o Android e a
/// fonte de verdade enquanto o campo esta focado; o TextEdit apenas recebe o
/// snapshot mais recente e continua podendo editar/tocar o cursor normalmente.
pub fn prepare_text_edit(ctx: &egui::Context, id: egui::Id, text: &mut String) -> bool {
    #[cfg(not(target_os = "android"))]
    {
        let _ = (ctx, id, text);
        false
    }

    #[cfg(target_os = "android")]
    {
        if ctx.memory(|memory| memory.focused()) != Some(id) {
            return false;
        }

        let incoming = {
            let Ok(mut bridge) = BRIDGE.lock() else {
                return false;
            };
            if bridge.field != Some(id) {
                return false;
            }
            let Some(incoming) = bridge.incoming.take() else {
                return false;
            };
            bridge.shadow = Some(incoming.clone());
            incoming
        };

        let changed = text.as_str() != incoming.text.as_str();
        if changed {
            text.clone_from(&incoming.text);
        }

        // Isto precisa acontecer antes de TextEdit::show: assim o proprio
        // widget parte do cursor que o IME escolheu, inclusive quando Gboard
        // move apenas a selecao sem alterar uma unica letra.
        set_egui_selection(ctx, id, text, incoming.selection);
        changed
    }
}

/// Sincroniza o lado egui DEPOIS que um TextEdit foi desenhado.
///
/// - no primeiro foco, semeia o InputConnection com o texto e cursor reais;
/// - depois do widget, se o usuario moveu o cursor/tocou no TextEdit, devolve
///   texto + selecao ao Android para a proxima operacao acontecer no ponto
///   correto.
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

            let state = state_from_egui(ctx, id, text, None);
            bridge.shadow = Some(state.clone());
            drop(bridge);

            configure_android(kind);
            send_to_android(&state);
            return false;
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

        false
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

#[cfg(any(target_os = "android", test))]
fn char_to_utf16_index(text: &str, char_index: usize) -> usize {
    text.chars().take(char_index).map(char::len_utf16).sum()
}

#[cfg(any(target_os = "android", test))]
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
