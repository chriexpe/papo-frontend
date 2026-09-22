//! Ponte de edicao de texto entre o egui e o GameTextInput do Android.
//!
//! O teclado Android edita um documento: texto completo, selecao/cursor e
//! regiao de composicao. O buffer precisa espelhar o TextEdit focado para que
//! recursos nativos como Backspace continuo, mover o cursor segurando espaco,
//! autocorrecao e insercao no meio da frase funcionem.
//!
//! O winit/eframe atual sobe o IME, mas nao encaminha esse estado ao egui.
//! A PapoActivity portanto envia o State por JNI e este modulo sincroniza os
//! dois lados sem fabricar eventos de tecla.

#[cfg(target_os = "android")]
use std::sync::{Mutex, OnceLock};

#[cfg(target_os = "android")]
use android_activity::{
    AndroidApp,
    input::{TextInputState, TextSpan},
};

#[cfg(target_os = "android")]
static APP: OnceLock<AndroidApp> = OnceLock::new();

#[cfg(target_os = "android")]
static BRIDGE: Mutex<Bridge> = Mutex::new(Bridge::new());

#[cfg(target_os = "android")]
#[derive(Clone, Debug, PartialEq, Eq)]
struct ImeState {
    text: String,
    // Offsets do Android sao unidades UTF-16.
    selection: (usize, usize),
    compose: Option<(usize, usize)>,
}

#[cfg(target_os = "android")]
struct Bridge {
    // TextEdit que atualmente e dono do documento do teclado.
    field: Option<egui::Id>,
    // Estado mais recente vindo da Activity, ainda nao aplicado ao egui.
    incoming: Option<ImeState>,
    // Ultimo estado em que Android e egui estavam sincronizados.
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

// Mantem a assinatura usada pelo raw_input_hook. A edicao propriamente dita
// acontece depois que cada TextEdit foi desenhado, em sync_text_edit.
#[cfg(target_os = "android")]
pub fn pump(ctx: &egui::Context, _raw_input: &mut egui::RawInput) {
    let focused = ctx.memory(|memory| memory.focused());
    if let Ok(mut bridge) = BRIDGE.lock()
        && bridge.field.is_some()
        && bridge.field != focused
    {
        bridge.clear();
    }
}

#[cfg(not(target_os = "android"))]
pub fn pump(_ctx: &egui::Context, _raw_input: &mut egui::RawInput) {}

/// Sincroniza um TextEdit com o documento mantido pelo IME.
///
/// Retorna true se o IME alterou o texto neste quadro.
pub fn sync_text_edit(
    ctx: &egui::Context,
    id: egui::Id,
    text: &mut String,
    response_has_focus: bool,
) -> bool {
    #[cfg(not(target_os = "android"))]
    {
        let _ = (ctx, id, text, response_has_focus);
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

        // Ao trocar de campo, o teclado recebe o documento REAL inteiro e a
        // selecao que o toque acabou de escolher no egui.
        if bridge.field != Some(id) {
            bridge.field = Some(id);
            bridge.incoming = None;
            let state = state_from_egui(ctx, id, text, None);
            bridge.shadow = Some(state.clone());
            drop(bridge);
            send_to_android(&state);
            return false;
        }

        let mut changed = false;

        // Enquanto o IME esta editando, seu State e atomico: texto, cursor e
        // composicao pertencem ao mesmo instante. Aplicar o pacote inteiro
        // evita a antiga divergencia em que o teclado achava que o cursor
        // estava no fim enquanto o egui o mostrava no meio.
        if let Some(incoming) = bridge.incoming.take() {
            if bridge.shadow.as_ref() != Some(&incoming) {
                if *text != incoming.text {
                    *text = incoming.text.clone();
                    changed = true;
                }
                set_egui_selection(ctx, id, text, incoming.selection);
                bridge.shadow = Some(incoming);
            }
        }

        // O caminho inverso cobre toque/mouse mudando o cursor, colar,
        // autocomplete/emoji do Papo e limpar a caixa depois de enviar.
        // Preservamos a regiao de composicao enquanto o texto nao mudou.
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
        if changed {
            ctx.request_repaint();
        }
        changed
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
    text.chars()
        .take(char_index)
        .map(char::len_utf16)
        .sum()
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
