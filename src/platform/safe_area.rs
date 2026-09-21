//! As bordas que o sistema ocupa na tela do Android.
//!
//! Do Android 15 em diante a janela é sempre de borda a borda: o relógio, a
//! barra de navegação e o recorte da câmera ficam **por cima** do que
//! desenhamos. Quem mede isso é a Activity, do lado Java, porque só ela
//! recebe o `WindowInsets` — e ela empurra os quatro números para cá.
//!
//! Guardar em atômicos deixa a leitura de graça: o desenho acontece numa
//! thread e a medida chega em outra, mas são quatro inteiros e nenhum
//! precisa concordar com o outro em tempo algum.

use std::sync::atomic::{AtomicI32, Ordering};

/// A janela, para conseguir pedir um quadro quando as bordas mudam. Sem
/// isto, fechar o teclado deixaria a caixa de escrever lá em cima até algo
/// não relacionado provocar o próximo quadro.
static REPAINT: std::sync::OnceLock<egui::Context> = std::sync::OnceLock::new();

static LEFT: AtomicI32 = AtomicI32::new(0);
static TOP: AtomicI32 = AtomicI32::new(0);
static RIGHT: AtomicI32 = AtomicI32::new(0);
static BOTTOM: AtomicI32 = AtomicI32::new(0);

/// Chamada pela `PapoActivity` sempre que as bordas mudam — ao abrir, ao
/// girar o aparelho e ao entrar ou sair do modo de tela cheia.
///
/// O nome é o que o JNI exige: `Java_` + o pacote e a classe com `_` no
/// lugar dos pontos + o nome do método. Mudar o pacote da Activity sem mudar
/// este nome faz o método sumir em tempo de execução.
///
/// # Safety
/// Chamada pelo JNI. Os dois primeiros parâmetros são o `JNIEnv` e a classe,
/// que não usamos — e por isso não há nada a desreferenciar aqui.
#[unsafe(no_mangle)]
pub extern "C" fn Java_io_github_chriexpe_papo_PapoActivity_nativeSetInsets(
    _env: *mut std::ffi::c_void,
    _class: *mut std::ffi::c_void,
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
) {
    LEFT.store(left, Ordering::Relaxed);
    TOP.store(top, Ordering::Relaxed);
    RIGHT.store(right, Ordering::Relaxed);
    BOTTOM.store(bottom, Ordering::Relaxed);
    log::debug!("bordas do sistema: {left} {top} {right} {bottom} (px)");
    if let Some(ctx) = REPAINT.get() {
        ctx.request_repaint();
    }
}

/// Guarda a janela na primeira vez que ela desenha. É de lá que sai o
/// pedido de quadro quando o teclado sobe ou desce.
pub fn install_repaint(ctx: &egui::Context) {
    if REPAINT.get().is_none() {
        let _ = REPAINT.set(ctx.clone());
    }
}

/// As bordas em pixels físicos, na ordem esquerda, cima, direita, baixo.
pub fn insets_px() -> (f32, f32, f32, f32) {
    (
        LEFT.load(Ordering::Relaxed) as f32,
        TOP.load(Ordering::Relaxed) as f32,
        RIGHT.load(Ordering::Relaxed) as f32,
        BOTTOM.load(Ordering::Relaxed) as f32,
    )
}
