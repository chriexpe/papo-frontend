//! Owns REST response values and problem envelopes; leaves transport encoding to HTTP serving.

use serde_json::{Value, json};

pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) body: Option<Value>,
    pub(crate) cookie: Option<String>,
    pub(crate) problem: bool,
    pub(crate) encoded: Option<Vec<u8>>,
    pub(crate) headers: Vec<(String, String)>,
}

impl Reply {
    pub(crate) fn json(status: u16, body: Value) -> Self {
        Self {
            status,
            body: Some(body),
            cookie: None,
            problem: false,
            encoded: None,
            headers: Vec::new(),
        }
    }

    pub(crate) fn empty() -> Self {
        Self {
            status: 204,
            body: None,
            cookie: None,
            problem: false,
            encoded: None,
            headers: Vec::new(),
        }
    }

    pub(crate) fn with_cookie(mut self, token: &str, max_age_seconds: u64) -> Self {
        self.cookie = Some(format!(
            "Auth={token}; Path=/; Max-Age={max_age_seconds}; HttpOnly; Secure; SameSite=Strict"
        ));
        self
    }

    pub(crate) fn clear_cookie(mut self) -> Self {
        self.cookie = Some("Auth=; Path=/; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; HttpOnly; Secure; SameSite=Strict".into());
        self
    }
}

pub(crate) fn problem(status: u16, slug: &str, path: &str) -> Reply {
    let (title, detail) = match slug {
        "invalid-param" => ("Parâmetro inválido", "campo ausente ou inválido"),
        "invalid-credentials" => ("Credenciais inválidas", "username ou senha incorretos"),
        "server-access-required" => (
            "Acesso ao servidor negado",
            "servidor não público: informe a senha do servidor em /auth/login_server antes de continuar",
        ),
        "unauthorized" => (
            "Token inválido ou expirado",
            "token de autenticação ausente, inválido ou expirado",
        ),
        "username-taken" => ("Username já existe", "o username informado já está em uso"),
        "not-found" => ("Recurso não encontrado", "servidor não encontrado"),
        _ => ("Erro interno", "falha injetada pelo cenário"),
    };
    let mut reply = Reply::json(
        status,
        json!({"type": format!("/errors/{slug}"), "title": title, "status": status, "detail": detail, "instance": path}),
    );
    reply.problem = true;
    reply
}

#[cfg(test)]
mod tests {
    //! Owns deterministic problem/cookie tests; leaves HTTP encoding to integration tests.

    use super::*;

    #[test]
    fn problems_preserve_status_slug_and_request_path() {
        let reply = problem(401, "unauthorized", "/auth/whoami");
        let body = reply.body.unwrap();
        assert_eq!(
            (
                reply.status,
                reply.problem,
                body["type"].as_str(),
                body["instance"].as_str()
            ),
            (
                401,
                true,
                Some("/errors/unauthorized"),
                Some("/auth/whoami")
            )
        );
    }
}
