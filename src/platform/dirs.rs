//! Onde o Papo guarda sessão e cache.
//!
//! Na área de trabalho quem decide é o XDG, via `directories`. No Android
//! não existe `~/.local/share`: cada aplicativo recebe do sistema uma pasta
//! só dele, e o caminho chega junto com a Activity — por isso ele é
//! guardado no início de `android_main` em vez de ser descoberto aqui.

use std::path::PathBuf;

#[cfg(not(target_os = "android"))]
pub fn data_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "papo").map(|dirs| dirs.data_dir().to_path_buf())
}

/// Caminho do banco de cache descartável. Fica na pasta privada de dados —
/// app-private no Android — e nunca guarda segredos.
pub fn cache_db() -> Option<PathBuf> {
    data_dir().map(|dir| dir.join("papo-cache.db"))
}

#[cfg(not(target_os = "android"))]
pub fn cache_dir() -> PathBuf {
    directories::ProjectDirs::from("", "", "papo")
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .unwrap_or_else(std::env::temp_dir)
}

#[cfg(target_os = "android")]
static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Guarda a pasta privada do aplicativo, que só a Activity sabe qual é.
#[cfg(target_os = "android")]
pub fn set_root(root: PathBuf) {
    if let Some(existing) = ROOT.get() {
        if existing != &root {
            log::error!(
                "app-private root divergiu: existente={} novo={}",
                existing.display(),
                root.display()
            );
        }
        return;
    }
    let _ = ROOT.set(root);
}

#[cfg(target_os = "android")]
pub fn data_dir() -> Option<PathBuf> {
    Some(ROOT.get()?.join("data"))
}

#[cfg(target_os = "android")]
pub fn cache_dir() -> PathBuf {
    ROOT.get()
        .map(|root| root.join("cache"))
        .unwrap_or_else(std::env::temp_dir)
}
