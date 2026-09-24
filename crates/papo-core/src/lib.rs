//! Núcleo compartilhado do cliente Papo.
//!
//! Não depende de egui, eframe ou GStreamer. Frontends diferentes podem
//! reutilizar o mesmo contrato REST/WebSocket, sessão e runtime assíncrono.

pub mod api;
pub mod cache;
pub mod core;
pub mod notification;
pub mod runtime;
pub mod state;
pub mod storage;

// PR7 feasibility probe. Test-only: it exercises the boring local-database
// subset Turso must support, and is not part of the production runtime.
#[cfg(test)]
mod turso_probe;

/// Chave estável e segura para nome de arquivo a partir do endereço de um
/// servidor. Separa sessão, cache e outras persistências por backend.
pub fn server_key(base_url: &str) -> String {
    let trimmed = base_url.trim().trim_end_matches('/').to_lowercase();
    let naked = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(&trimmed);

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in trimmed.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    let readable: String = naked
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(40)
        .collect();
    format!("{}-{hash:016x}", readable.trim_matches('-'))
}

#[cfg(test)]
mod tests {
    use super::server_key;

    #[test]
    fn enderecos_diferentes_geram_chaves_diferentes() {
        assert_ne!(server_key("https://um.example"), server_key("https://dois.example"));
    }

    #[test]
    fn barra_final_e_caixa_nao_mudam_a_chave() {
        assert_eq!(
            server_key("https://Papo.Example/"),
            server_key("https://papo.example")
        );
    }

    #[test]
    fn chave_serve_de_nome_de_arquivo() {
        let key = server_key("https://papo.example:8080/base");
        assert!(key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }
}
