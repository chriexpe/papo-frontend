#[cfg(target_os = "linux")]
pub mod activate;
#[cfg(target_os = "linux")]
pub mod appmenu;
pub mod autostart;

/// O Papo está rodando empacotado no Flatpak? Vale para o que o sandbox
/// muda: o autostart chama o `flatpak run`, e a bandeja abre mão do nome
/// próprio no barramento, que o sandbox não deixa registrar.
pub fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}
#[cfg(target_os = "linux")]
pub mod blur;
pub mod desktop;
/// Onde ficam sessão e cache em cada plataforma.
pub mod dirs;
pub mod files;
#[cfg(target_os = "linux")]
pub mod global_menu;
pub mod menu;
/// Bordas do sistema no Android (barra de status, navegação, recorte).
#[cfg(target_os = "android")]
pub mod safe_area;
#[cfg(target_os = "linux")]
pub mod kwin;
#[cfg(target_os = "linux")]
pub mod launcher;
#[cfg(target_os = "linux")]
pub mod notify;
#[cfg(target_os = "linux")]
pub mod tray;
