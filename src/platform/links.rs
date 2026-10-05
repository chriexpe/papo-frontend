//! Abrir um endereço no navegador do sistema.
//!
//! O `ctx.open_url` do egui depende da feature `links` do egui-winit, que o
//! Papo nunca ligou: o pedido era descartado em silêncio e o botão "Abrir" do
//! aviso de link não fazia nada. Aqui cada plataforma usa o seu próprio
//! caminho.

/// Só web e `mailto:`: o texto vem de mensagens de terceiros, e esquemas como `file:`,
/// `intent:` ou `javascript:` nunca devem sair daqui.
pub fn is_openable(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    if let Some(address) = lower.strip_prefix("mailto:") {
        return !address.is_empty();
    }
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && url.split("://").nth(1).is_some_and(|rest| !rest.is_empty())
}

/// Entrega o endereço ao navegador padrão. Devolve se o pedido foi feito.
pub fn open_url(url: &str) -> bool {
    if !is_openable(url) {
        log::warn!("endereço recusado: {url}");
        return false;
    }
    let opened = launch(url);
    if !opened {
        log::warn!("não deu para abrir {url}");
    }
    opened
}

#[cfg(target_os = "android")]
fn launch(url: &str) -> bool {
    super::jvm::call_activity("openUrl", "(Ljava/lang/String;)V", Some(url))
}

#[cfg(target_os = "windows")]
fn launch(url: &str) -> bool {
    std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .spawn()
        .is_ok()
}

#[cfg(all(unix, not(target_os = "android")))]
fn launch(url: &str) -> bool {
    // Sem shell: o endereço vai como um único argumento. No Flatpak o
    // `xdg-open` fala com o portal.
    std::process::Command::new("xdg-open")
        .arg(url)
        .spawn()
        .is_ok()
}

#[cfg(not(any(unix, target_os = "windows")))]
fn launch(_url: &str) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::is_openable;

    #[test]
    fn only_plain_web_addresses_open() {
        assert!(is_openable("https://exemplo.com/a?b=1"));
        assert!(is_openable("HTTP://exemplo.com"));
        assert!(is_openable("mailto:ana@exemplo.com"));
        assert!(!is_openable("mailto:"));
        assert!(!is_openable("file:///etc/passwd"));
        assert!(!is_openable("javascript:alert(1)"));
        assert!(!is_openable("intent://x#Intent;end"));
        assert!(!is_openable("https://"));
        assert!(!is_openable("https://a.com/ b"));
        assert!(!is_openable("-oProxyCommand=x"));
    }
}
