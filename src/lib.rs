//! O Papo propriamente dito.
//!
//! A área de trabalho executa um binário e o Android carrega uma biblioteca,
//! então o que vale nos dois é esta biblioteca: ela guarda a aplicação
//! inteira. O `src/main.rs` é só a porta de entrada da área de trabalho, e
//! `android::android_main` é a do Android.

pub mod api;
pub mod app;
pub mod i18n;
pub mod media;
pub mod platform;
pub mod state;
pub mod storage;
pub mod ui;
pub mod voice;
pub mod webembed;

#[cfg(target_os = "android")]
mod android;

/// Identificador do aplicativo na área de trabalho.
///
/// É o `app_id` do Wayland, o nome do `.desktop` e o do ícone. O compositor
/// liga a janela ao lançador por este nome, então os três têm de ser o mesmo
/// — inclusive dentro do Flatpak, onde o identificador é o do pacote.
///
/// No Android **não** vale este: lá o nome é o `applicationId` do Gradle,
/// todo em minúsculas (`io.github.chriexpe.papo`). As convenções dos dois
/// mundos se contradizem — o AppStream capitaliza a última parte, o Android
/// não — e nenhuma string agrada às duas.
pub const APP_ID: &str = "io.github.chriexpe.Papo";
