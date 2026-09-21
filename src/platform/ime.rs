//! A ponte entre o teclado do Android e o egui.
//!
//! O winit abre e fecha o teclado do Android, mas **não** entrega o que se
//! digita nele: ele trata dezesseis eventos do `android-activity` e
//! `TextInputEvent` não é um deles. O teclado sobe e as letras não chegam a
//! lugar nenhum.
//!
//! Quem guarda o texto é o `GameTextInput`, do lado do GameActivity. Ler
//! esse buffer pelo `AndroidApp::text_input_state()` **derruba o
//! aplicativo**: o `android-activity` 0.6.1 monta a fatia com
//! `slice::from_raw_parts` sobre o ponteiro do texto sem conferir se ele é
//! nulo, e ele é nulo enquanto ninguém digitou nada. Ponteiro nulo viola a
//! pré-condição mesmo com tamanho zero, e o pânico que sai daí não desenrola
//! — aborta. Era o que derrubava o aplicativo assim que o teclado subia.
//!
//! Então o texto vem pelo outro lado: a `PapoActivity` recebe o
//! `stateChanged` do `GameTextInput` e o empurra para cá. Não se olha nada a
//! cada quadro, a letra chega na hora, e o caminho não passa perto da
//! função quebrada.
//!
//! A conta é por prefixo comum: o que sumiu do fim vira `Backspace`, o que
//! apareceu vira `Text`. É o bastante para digitar, apagar e para a troca
//! que o corretor automático faz ao fechar uma palavra.

#[cfg(target_os = "android")]
use std::sync::{Mutex, OnceLock};

#[cfg(target_os = "android")]
use android_activity::AndroidApp;
#[cfg(target_os = "android")]
use android_activity::input::TextInputState;

#[cfg(target_os = "android")]
static APP: OnceLock<AndroidApp> = OnceLock::new();
#[cfg(target_os = "android")]
static BRIDGE: Mutex<Bridge> = Mutex::new(Bridge::new());

#[cfg(target_os = "android")]
struct Bridge {
    /// Quem tinha o foco no quadro anterior.
    focus: Option<egui::Id>,
    /// O que o teclado mandou e ainda não virou evento.
    incoming: Option<String>,
    /// O que já virou evento. É o espelho do buffer do teclado, não do campo
    /// do egui — a diferença entre os dois é o que vira evento.
    mirror: String,
}

#[cfg(target_os = "android")]
impl Bridge {
    const fn new() -> Self {
        Self {
            focus: None,
            incoming: None,
            mirror: String::new(),
        }
    }
}

/// Guarda a Activity, que é por onde se zera o teclado ao trocar de campo.
#[cfg(target_os = "android")]
pub fn install(app: AndroidApp) {
    let _ = APP.set(app);
}

/// Recebe o texto do teclado, vindo da `PapoActivity`.
///
/// O nome é o que o JNI exige: `Java_` + o pacote e a classe com `_` no
/// lugar dos pontos + o nome do método. Mudar o pacote da Activity sem mudar
/// este nome faz o método sumir em tempo de execução.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeSetText(
    mut env: jni::JNIEnv,
    _class: jni::objects::JClass,
    text: jni::objects::JString,
) {
    let Ok(text) = env.get_string(&text) else {
        return;
    };
    let text: String = text.into();
    if let Ok(mut bridge) = BRIDGE.lock() {
        bridge.incoming = Some(text);
    }
    // Chegou de uma thread do Java: sem este pedido a letra esperaria um
    // quadro que viria só por outro motivo.
    super::wake::request();
}

/// Converte o que o teclado mandou em eventos do egui.
#[cfg(target_os = "android")]
pub fn pump(ctx: &egui::Context, raw_input: &mut egui::RawInput) {
    let Ok(mut bridge) = BRIDGE.lock() else {
        return;
    };

    // Trocou de campo (ou perdeu o foco): o buffer do teclado volta a zero,
    // senão o texto do campo anterior contaria como já digitado aqui.
    let focus = ctx.memory(|memory| memory.focused());
    if bridge.focus != focus {
        bridge.focus = focus;
        bridge.mirror.clear();
        bridge.incoming = None;
        if let Some(app) = APP.get() {
            // Escrever é seguro; é só a leitura que está quebrada.
            app.set_text_input_state(TextInputState::default());
        }
        return;
    }
    if focus.is_none() {
        return;
    }

    let Some(current) = bridge.incoming.take() else {
        return;
    };
    if current == bridge.mirror {
        return;
    }

    let common = common_prefix(&bridge.mirror, &current);
    let removed = bridge.mirror.chars().count() - common;
    for _ in 0..removed {
        push_backspace(raw_input);
    }
    let inserted: String = current.chars().skip(common).collect();

    // Só os tamanhos: pelo mesmo caminho passa o campo de senha, e o logcat
    // é lido por qualquer um com o cabo na mão.
    log::debug!(
        "teclado: {removed} apagado(s), {} escrito(s)",
        inserted.chars().count()
    );

    if !inserted.is_empty() {
        raw_input.events.push(egui::Event::Text(inserted));
    }

    bridge.mirror = current;
}

/// Quantos caracteres os dois textos têm em comum, do início.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn common_prefix(before: &str, after: &str) -> usize {
    before
        .chars()
        .zip(after.chars())
        .take_while(|(a, b)| a == b)
        .count()
}

#[cfg(target_os = "android")]
fn push_backspace(raw_input: &mut egui::RawInput) {
    for pressed in [true, false] {
        raw_input.events.push(egui::Event::Key {
            key: egui::Key::Backspace,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::common_prefix;

    #[test]
    fn conta_o_prefixo_em_caracteres_nao_em_bytes() {
        assert_eq!(common_prefix("ola", "olar"), 3);
        assert_eq!(common_prefix("ola", "ol"), 2);
        assert_eq!(common_prefix("", "a"), 0);
        assert_eq!(common_prefix("abc", "xyz"), 0);
    }

    #[test]
    fn acentos_contam_como_um_caractere_so() {
        // "ação" tem 4 caracteres em 5 bytes: o "ç" ocupa dois. Contando
        // bytes daríamos um `Backspace` a mais e a palavra perderia letra.
        assert_eq!("ação".chars().count(), 4);
        assert_eq!("ação".len(), 6);
        assert_eq!(common_prefix("ação", "ação!"), 4);
    }

    #[test]
    fn corretor_que_troca_o_meio_da_palavra_apaga_so_o_que_mudou() {
        // É o que o teclado faz ao fechar a palavra: "acao" vira "ação".
        // Só o que vem depois do prefixo comum é reescrito.
        assert_eq!(common_prefix("acao", "ação"), 1);
    }
}
