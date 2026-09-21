//! A ponte entre o teclado do Android e o egui.
//!
//! O winit abre e fecha o teclado do Android, mas **não** entrega o que se
//! digita nele: ele trata dezesseis eventos do `android-activity` e
//! `TextInputEvent` não é um deles. O resultado é o teclado subir e as
//! letras não chegarem a lugar nenhum.
//!
//! Quem guarda o texto é o `GameTextInput`, do lado do GameActivity. Aqui
//! olhamos esse buffer a cada quadro e transformamos a diferença em eventos
//! que o egui entende — como se alguém tivesse digitado num teclado comum.
//! Assim nenhum campo de texto da interface precisou mudar.
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

/// De quanto em quanto tempo olhamos o buffer do teclado enquanto há campo
/// em foco. A 60 Hz o atraso não dá para perceber, e fora do foco não se
/// pede quadro nenhum — a bateria só paga enquanto se está digitando.
#[cfg(target_os = "android")]
const POLL: std::time::Duration = std::time::Duration::from_millis(16);

#[cfg(target_os = "android")]
static APP: OnceLock<AndroidApp> = OnceLock::new();
#[cfg(target_os = "android")]
static BRIDGE: Mutex<Bridge> = Mutex::new(Bridge::new());

#[cfg(target_os = "android")]
struct Bridge {
    /// Quem tinha o foco no quadro anterior.
    focus: Option<egui::Id>,
    /// O que o `GameTextInput` tinha da última vez que olhamos. É o espelho
    /// do buffer do teclado, não do campo do egui — a diferença entre os
    /// dois é justamente o que vira evento.
    mirror: String,
}

#[cfg(target_os = "android")]
impl Bridge {
    const fn new() -> Self {
        Self {
            focus: None,
            mirror: String::new(),
        }
    }
}

/// Guarda a Activity, que é por onde se fala com o teclado.
#[cfg(target_os = "android")]
pub fn install(app: AndroidApp) {
    let _ = APP.set(app);
}

/// Converte o que mudou no teclado em eventos do egui.
#[cfg(target_os = "android")]
pub fn pump(ctx: &egui::Context, raw_input: &mut egui::RawInput) {
    let Some(app) = APP.get() else {
        return;
    };
    let Ok(mut bridge) = BRIDGE.lock() else {
        return;
    };

    // Trocou de campo (ou perdeu o foco): o buffer do teclado volta a zero,
    // senão o texto do campo anterior contaria como já digitado aqui.
    let focus = ctx.memory(|memory| memory.focused());

    // O egui só desenha quando alguém pede, e a chegada de texto não pede
    // nada: o winit ignora o `TextInputEvent`, então não há evento nenhum
    // para acordar o laço. Sem isto a letra só aparece no próximo quadro que
    // acontecesse por outro motivo — um toque, uma animação — e digitar fica
    // com atraso. Enquanto houver campo em foco, pedimos o quadro seguinte.
    if focus.is_some() {
        ctx.request_repaint_after(POLL);
    }

    if bridge.focus != focus {
        bridge.focus = focus;
        bridge.mirror.clear();
        app.set_text_input_state(TextInputState::default());
        return;
    }
    if focus.is_none() {
        return;
    }

    let current = app.text_input_state();
    if current.text == bridge.mirror {
        return;
    }

    let common = common_prefix(&bridge.mirror, &current.text);
    let removed = bridge.mirror.chars().count() - common;
    for _ in 0..removed {
        push_backspace(raw_input);
    }
    let inserted: String = current.text.chars().skip(common).collect();

    // Só os tamanhos: pelo campo de senha passa o mesmo caminho, e o logcat
    // é lido por qualquer um com o cabo na mão.
    log::debug!(
        "teclado: {removed} apagado(s), {} escrito(s)",
        inserted.chars().count()
    );

    if !inserted.is_empty() {
        raw_input.events.push(egui::Event::Text(inserted));
    }
    bridge.mirror = current.text;
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
