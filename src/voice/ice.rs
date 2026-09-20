//! Servidores ICE, do jeito que o backend entrega para o jeito que o
//! `webrtcbin` aceita.
//!
//! O backend devolve o mesmo formato do navegador (`urls`, `username`,
//! `credential`), mas o `webrtcbin` não tem campo de usuário e senha: a
//! credencial vai dentro do endereço, como `turn://usuário:senha@servidor`.
//! A senha do TURN é um HMAC em Base64, cheia de `+`, `/` e `=`, então
//! precisa ser escapada — sem isso o coturn recusa a autenticação e a call
//! atravessa NAT nenhum.

use crate::api::models::IceServer;

/// Endereços prontos para o `webrtcbin`: um STUN (ele só aceita um) e
/// quantos TURN houver.
#[derive(Debug, Clone, Default)]
pub struct IceConfig {
    pub stun: Option<String>,
    pub turn: Vec<String>,
}

impl IceConfig {
    pub fn from_servers(servers: &[IceServer]) -> Self {
        let mut config = Self::default();
        for server in servers {
            for url in &server.urls {
                let url = url.trim();
                if url.starts_with("stun:") {
                    if config.stun.is_none() {
                        config.stun = Some(stun_url(url));
                    }
                } else if url.starts_with("turn:") || url.starts_with("turns:") {
                    if let Some(turn) = turn_url(
                        url,
                        server.username.as_deref(),
                        server.credential.as_deref(),
                    ) {
                        config.turn.push(turn);
                    }
                }
            }
        }
        config
    }
}

/// `stun:servidor:porta` → `stun://servidor:porta`.
fn stun_url(url: &str) -> String {
    match url.split_once(':') {
        Some((scheme, rest)) => format!("{scheme}://{rest}"),
        None => url.to_owned(),
    }
}

/// `turn:servidor:porta?transport=udp` + credencial → o endereço com
/// usuário e senha embutidos, escapados.
fn turn_url(url: &str, username: Option<&str>, credential: Option<&str>) -> Option<String> {
    let (scheme, rest) = url.split_once(':')?;
    let (user, secret) = (username?, credential?);
    Some(format!(
        "{scheme}://{}:{}@{rest}",
        escape(user),
        escape(secret)
    ))
}

/// Escapa o que não pode aparecer cru na parte de credencial de um
/// endereço. A lista de caracteres livres é a do RFC 3986 para `userinfo`,
/// sem os separadores `:` e `@`.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(urls: &[&str], user: Option<&str>, secret: Option<&str>) -> IceServer {
        IceServer {
            urls: urls.iter().map(|u| (*u).to_owned()).collect(),
            username: user.map(str::to_owned),
            credential: secret.map(str::to_owned),
        }
    }

    #[test]
    fn stun_ganha_as_duas_barras() {
        let config = IceConfig::from_servers(&[server(&["stun:stun.l.google.com:19302"], None, None)]);
        assert_eq!(config.stun.as_deref(), Some("stun://stun.l.google.com:19302"));
    }

    /// A credencial do TURN é Base64 de um HMAC: sem escapar, um `+` viraria
    /// espaço e um `/` cortaria o endereço no meio.
    #[test]
    fn credencial_do_turn_vai_escapada() {
        let config = IceConfig::from_servers(&[server(
            &["turn:papo.example:3478?transport=udp"],
            Some("1790000000:u-1"),
            Some("ab+cd/ef="),
        )]);
        assert_eq!(
            config.turn,
            vec!["turn://1790000000%3Au-1:ab%2Bcd%2Fef%3D@papo.example:3478?transport=udp"]
        );
    }

    /// Sem TURN configurado o servidor manda só o STUN; a sala funciona em
    /// rede sem NAT restritivo, que é o caso do self-hosted na LAN.
    #[test]
    fn turn_sem_credencial_fica_de_fora() {
        let config = IceConfig::from_servers(&[server(&["turn:papo.example:3478"], None, None)]);
        assert!(config.turn.is_empty());
    }
}
