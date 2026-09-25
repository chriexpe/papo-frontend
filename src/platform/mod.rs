#[cfg(target_os = "linux")]
pub mod activate;
#[cfg(target_os = "linux")]
pub mod appmenu;
pub mod autostart;
pub mod client_db;

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
/// ConnectivityManager/lifecycle -> runtimes de rede. Também compila em
/// testes desktop para validar fanout/registro sem depender de um aparelho.
#[cfg(any(target_os = "android", test))]
pub mod android_network;
/// Process-wide foreground/headless runtime exclusion.
#[cfg(any(target_os = "android", test))]
pub mod runtime_lease;
/// `onTrimMemory` -> latch coalescido -> política de mídia do PR36.
#[cfg(any(target_os = "android", test))]
pub mod memory_pressure;
/// Foreground service, lifecycle e Picture-in-Picture da call.
#[cfg(target_os = "android")]
pub mod android_call;
/// Notificações nativas de mensagens e navegação por toque.
#[cfg(target_os = "android")]
pub mod android_message;
/// Persistent WorkManager scheduling and cold headless JNI entry points.
#[cfg(target_os = "android")]
pub mod android_work;
/// Browser surface nativa para embeds ricos.
#[cfg(target_os = "android")]
pub mod android_webembed;
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
/// Toolkit-free WPE WebKit runtime/page bridge for Linux WebEmbed.
#[cfg(target_os = "linux")]
pub mod linux_wpe;
/// WPE WebKit WebEmbed backend and DMA-BUF → Glow importer.
#[cfg(target_os = "linux")]
pub mod linux_webembed;
#[cfg(target_os = "linux")]
pub mod kwin;
#[cfg(target_os = "linux")]
pub mod launcher;
#[cfg(target_os = "linux")]
pub mod notify;
#[cfg(target_os = "linux")]
pub mod tray;
