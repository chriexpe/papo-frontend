//! Copiar texto para a área de transferência.
//!
//! No desktop o `ctx.copy_text` do egui chega ao sistema. No Android não: o
//! egui-winit só tem a área de transferência de dentro do app, então "Copiar"
//! parecia funcionar, mas nada podia ser colado em outro aplicativo. Lá o
//! texto também vai para o `ClipboardManager` da Activity.

pub fn text(ctx: &egui::Context, text: impl Into<String>) {
    let text = text.into();
    #[cfg(target_os = "android")]
    if !super::jvm::call_activity("copyText", "(Ljava/lang/String;)V", Some(&text)) {
        log::warn!("Android não copiou o texto");
    }
    ctx.copy_text(text);
}
