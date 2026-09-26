//! Acordar a janela de fora do laço de quadros.
//!
//! No Android há coisas que chegam pela Activity, numa thread do Java: as
//! bordas do sistema quando o teclado sobe, e o próprio texto digitado. O
//! egui só desenha quando alguém pede, e nenhuma delas pediria sozinha — a
//! mudança ficaria esperando um quadro que só viria por outro motivo.

use std::sync::OnceLock;

static WINDOW: OnceLock<egui::Context> = OnceLock::new();

/// Guarda a janela na primeira vez que ela desenha.
pub fn install(ctx: &egui::Context) {
    if WINDOW.get().is_none() {
        let _ = WINDOW.set(ctx.clone());
    }
}

/// Pede o próximo quadro. Pode ser chamada de qualquer thread.
pub fn request() {
    if let Some(ctx) = WINDOW.get() {
        ctx.request_repaint();
    }
}
