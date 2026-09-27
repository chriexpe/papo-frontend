//! O voltar do sistema fecha o que está aberto por cima da conversa.
//!
//! O Android só entrega o voltar a quem pediu antes: o `OnBackPressedCallback`
//! da Activity fica desligado enquanto não há nada para fechar, e aí o
//! sistema faz o de sempre (inclusive a animação preditiva). A interface diz
//! a cada quadro se há algo aberto; só a mudança atravessa o JNI.
//!
//! O resto é igual ao `memory_pressure`: a thread Java só marca um latch e
//! acorda a janela, e quem fecha é o quadro seguinte.

use std::sync::atomic::{AtomicBool, Ordering};

static PRESSED: AtomicBool = AtomicBool::new(false);
static INTERCEPTING: AtomicBool = AtomicBool::new(false);

/// Há algo que o voltar deve fechar neste quadro? Barato: só chama a
/// Activity quando a resposta muda.
pub fn intercept(open: bool) {
    if INTERCEPTING.swap(open, Ordering::AcqRel) != open {
        #[cfg(target_os = "android")]
        super::jvm::call_activity_flag("setBackIntercept", open);
    }
    if !open {
        // Um voltar que chegou quando nada mais estava aberto não vale para
        // o próximo cartão.
        PRESSED.store(false, Ordering::Release);
    }
}

/// O voltar foi apertado desde o último quadro. Consome o aviso.
pub fn take() -> bool {
    PRESSED.swap(false, Ordering::AcqRel)
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_chriexpe_papo_PapoActivity_nativeBackPressed(
    _env: jni::JNIEnv,
    _class: jni::objects::JClass,
) {
    PRESSED.store(true, Ordering::Release);
    super::wake::request();
}
