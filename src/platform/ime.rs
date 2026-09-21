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
//! — aborta. Por isso o texto vem empurrado pela `PapoActivity`, que recebe
//! o `stateChanged` do próprio `GameTextInput`.
//!
//! ## Os dois textos
//!
//! O buffer do teclado e o campo do egui são textos **diferentes**. O
//! teclado não sabe o que já estava escrito no campo, e o egui não sabe o
//! que o teclado guarda. Tudo o que esta ponte faz é olhar o que mudou de um
//! lado e contar isso ao outro.
//!
//! Daí o enchimento: o buffer começa com [`PAD`] espaços, e o cursor atrás
//! deles. Sem isso, quando o que foi digitado nesta sessão acabasse o
//! teclado não teria mais o que apagar — pararia de avisar, e segurar o
//! apagar deixaria de apagar, apesar de ainda haver texto no campo. Com o
//! enchimento sempre sobra o que comer: cada espaço comido é um `Backspace`
//! de verdade no campo, e o enchimento é reposto.
//!
//! E daí também a desconfiança: um estado que **não** comece pelo nosso
//! enchimento não veio do nosso buffer. É o campo anterior chegando
//! atrasado, logo depois de trocar de foco. Confiar nele escrevia o conteúdo
//! do campo antigo dentro do novo por um instante.

#[cfg(target_os = "android")]
use std::sync::{Mutex, OnceLock};

#[cfg(target_os = "android")]
use android_activity::AndroidApp;
#[cfg(target_os = "android")]
use android_activity::input::{TextInputState, TextSpan};

/// Quantos espaços ficam à esquerda do que se digita, como reserva para o
/// apagar. Oito é folga de sobra: o enchimento é reposto a cada vez que o
/// teclado encosta nele, então nunca se come mais do que um ou dois.
const PAD: usize = 8;

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
    /// O espelho do buffer do teclado — enchimento incluído. Não é o texto
    /// do campo: é o que o teclado tinha da última vez que olhamos.
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

/// Guarda a Activity, que é por onde se escreve no buffer do teclado.
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

/// Põe o buffer do teclado de volta em "só o enchimento", com o cursor no
/// fim, e passa a esperar exatamente isso de volta.
#[cfg(target_os = "android")]
fn prime(bridge: &mut Bridge) {
    let text = " ".repeat(PAD);
    bridge.mirror = text.clone();
    bridge.incoming = None;
    if let Some(app) = APP.get() {
        // Escrever é seguro; é só a leitura do `android-activity` que está
        // quebrada.
        app.set_text_input_state(TextInputState {
            selection: TextSpan {
                start: PAD,
                end: PAD,
            },
            compose_region: None,
            text,
        });
    }
}

/// Converte o que o teclado mandou em eventos do egui.
#[cfg(target_os = "android")]
pub fn pump(ctx: &egui::Context, raw_input: &mut egui::RawInput) {
    let Ok(mut bridge) = BRIDGE.lock() else {
        return;
    };

    // Trocou de campo (ou perdeu o foco): o buffer volta ao enchimento,
    // senão o que foi digitado no campo anterior contaria como digitado aqui.
    let focus = ctx.memory(|memory| memory.focused());
    if bridge.focus != focus {
        bridge.focus = focus;
        prime(&mut bridge);
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

    let kept = leading_pad(&current);

    if kept == 0 {
        // Não começa pelo nosso enchimento: não veio do nosso buffer. É o
        // campo anterior chegando atrasado. Repor e não inventar edição
        // nenhuma — era isto que piscava o texto antigo dentro do campo novo.
        log::debug!("teclado: estado de fora do buffer, ignorado");
        prime(&mut bridge);
        return;
    }

    if kept < PAD {
        // O apagar passou do que foi digitado e comeu parte do enchimento.
        // Cada espaço comido é um apagar de verdade no campo — é isto que
        // faz segurar o apagar continuar apagando.
        let typed = bridge.mirror.chars().count().saturating_sub(PAD);
        let removed = typed + (PAD - kept);
        for _ in 0..removed {
            push_backspace(raw_input);
        }
        log::debug!("teclado: {removed} apagado(s), 0 escrito(s)");
        prime(&mut bridge);
        return;
    }

    // Enchimento inteiro: a diferença está no que veio depois dele.
    let before: String = bridge.mirror.chars().skip(PAD).collect();
    let after: String = current.chars().skip(PAD).collect();
    let common = common_prefix(&before, &after);
    let removed = before.chars().count() - common;
    for _ in 0..removed {
        push_backspace(raw_input);
    }
    let inserted: String = after.chars().skip(common).collect();

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

/// Quantos espaços de enchimento sobraram no começo, no máximo [`PAD`].
///
/// Passar de [`PAD`] seria o próprio usuário tendo digitado um espaço logo
/// no começo; esse espaço é dele, não nosso.
#[cfg_attr(not(target_os = "android"), allow(dead_code))]
fn leading_pad(text: &str) -> usize {
    text.chars().take_while(|c| *c == ' ').count().min(PAD)
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
    fn o_enchimento_e_contado_ate_o_limite_e_nao_alem() {
        use super::{PAD, leading_pad};

        // Buffer intocado: o enchimento inteiro.
        assert_eq!(leading_pad(&" ".repeat(PAD)), PAD);
        // Com texto digitado atrás dele, continua inteiro.
        assert_eq!(leading_pad(&format!("{}ola", " ".repeat(PAD))), PAD);
        // O apagar comeu um: é um apagar de verdade no campo.
        assert_eq!(leading_pad(&" ".repeat(PAD - 1)), PAD - 1);
        // Um espaço digitado pelo usuário é dele, não nosso: a conta para
        // no limite, senão o espaço sumiria em vez de ser escrito.
        assert_eq!(leading_pad(&" ".repeat(PAD + 3)), PAD);
        // Texto que não começa pelo enchimento não veio do nosso buffer.
        assert_eq!(leading_pad("senha123"), 0);
        assert_eq!(leading_pad(""), 0);
    }

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
