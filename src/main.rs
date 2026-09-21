#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Porta de entrada da área de trabalho.
//!
//! O Papo mora na biblioteca (`src/lib.rs`), que é o que o Android carrega.
//! Aqui ficam só o executável e os comandos de linha que não fazem sentido
//! no celular — `selftest`, `media-test`, `voice-check` e companhia.

#[cfg(not(target_os = "android"))]
mod cli;

#[cfg(not(target_os = "android"))]
fn main() -> eframe::Result<()> {
    cli::main()
}

/// No Android quem abre a janela é o `android_main` da biblioteca; este
/// executável existe só para o `cargo` ter o que construir.
#[cfg(target_os = "android")]
fn main() {}
