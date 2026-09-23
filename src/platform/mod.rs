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
/// Chamar a Activity a partir do Rust (Android).
#[cfg(target_os = "android")]
pub mod jvm;
/// Foreground service, lifecycle e Picture-in-Picture da call.
#[cfg(target_os = "android")]
pub mod android_call;
/// Notificações nativas de mensagens e navegação por toque.
#[cfg(target_os = "android")]
pub mod android_message;
/// Permissões do Android, pedidas quando fazem falta.
#[cfg(target_os = "android")]
pub mod permission;
pub mod menu;
/// Acordar a janela de fora do laço de quadros (Android).
#[cfg(target_os = "android")]
pub mod wake;
/// A ponte legada entre o teclado do Android e TextEdit. Continua servindo
/// campos simples enquanto compositor/edição usam um EditText nativo.
pub mod ime;
/// Editor Android de verdade, sobreposto à superfície do egui.
#[cfg(target_os = "android")]
pub mod native_text;
/// Campos Android comuns, cada um com EditText e estado próprios.
#[cfg(target_os = "android")]
pub mod native_field;
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
