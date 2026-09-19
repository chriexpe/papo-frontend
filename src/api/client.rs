//! Cliente REST.
//!
//! A sessão vive num cookie `Auth` HttpOnly. Num cliente nativo não há CORS
//! nem armazenamento do navegador: guardamos o cookie num pote próprio e o
//! gravamos em disco para sobreviver ao fechamento da janela.

use std::sync::{Arc, RwLock};

use reqwest::header::{HeaderValue, COOKIE};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use url::Url;

use super::models::*;

const COOKIE_NAME: &str = "Auth";

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    Problem(String),
    #[error("sessão expirada")]
    Unauthorized,
    #[error("recurso não encontrado")]
    NotFound,
    #[error("falha de rede: {0}")]
    Network(String),
    #[error("resposta inesperada: {0}")]
    Decode(String),
}

pub type ApiResult<T> = Result<T, ApiError>;

/// Pote de cookies mínimo: o backend só entrega o cookie `Auth`.
#[derive(Default, Debug)]
pub struct Session {
    token: RwLock<Option<String>>,
}

impl Session {
    pub fn token(&self) -> Option<String> {
        self.token.read().ok().and_then(|token| token.clone())
    }

    pub fn set_token(&self, token: Option<String>) {
        if let Ok(mut slot) = self.token.write() {
            *slot = token;
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.token().is_some()
    }

    /// Extrai o cookie `Auth` de um cabeçalho `Set-Cookie`.
    fn absorb(&self, response: &reqwest::Response) {
        for value in response.headers().get_all(reqwest::header::SET_COOKIE) {
            let Ok(value) = value.to_str() else { continue };
            let Some(pair) = value.split(';').next() else {
                continue;
            };
            let Some((name, token)) = pair.split_once('=') else {
                continue;
            };
            if name.trim() != COOKIE_NAME {
                continue;
            }
            // Valor vazio com Expires no passado significa logout.
            self.set_token((!token.is_empty()).then(|| token.to_owned()));
        }
    }
}

#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    base: Url,
    pub session: Arc<Session>,
}

impl Api {
    pub fn new(base_url: &str, session: Arc<Session>) -> ApiResult<Self> {
        let base = Url::parse(base_url).map_err(|e| ApiError::Network(e.to_string()))?;
        let http = reqwest::Client::builder()
            .user_agent(concat!("papo/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| ApiError::Network(e.to_string()))?;
        Ok(Self {
            http,
            base,
            session,
        })
    }

    /// Endereço do WebSocket derivado da base (`https` → `wss`).
    pub fn websocket_url(&self) -> String {
        let mut url = self.base.clone();
        let scheme = if url.scheme() == "http" { "ws" } else { "wss" };
        let _ = url.set_scheme(scheme);
        url.set_path("/ws");
        url.to_string()
    }

    async fn send<B: Serialize>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
    ) -> ApiResult<reqwest::Response> {
        let url = self
            .base
            .join(path)
            .map_err(|e| ApiError::Network(e.to_string()))?;
        let mut request = self.http.request(method, url);
        if let Some(token) = self.session.token() {
            let cookie = format!("{COOKIE_NAME}={token}");
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                request = request.header(COOKIE, value);
            }
        }
        if let Some(body) = body {
            request = request.json(body);
        }

        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        self.session.absorb(&response);
        Ok(response)
    }

    async fn parse<T: DeserializeOwned>(response: reqwest::Response) -> ApiResult<T> {
        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;

        if status.is_success() {
            if body.trim().is_empty() {
                return serde_json::from_str("null").map_err(|e| ApiError::Decode(e.to_string()));
            }
            return serde_json::from_str(&body).map_err(|e| ApiError::Decode(e.to_string()));
        }

        match status {
            StatusCode::UNAUTHORIZED => Err(ApiError::Unauthorized),
            StatusCode::NOT_FOUND => Err(ApiError::NotFound),
            _ => {
                let problem: Problem = serde_json::from_str(&body).unwrap_or_default();
                Err(ApiError::Problem(if problem.status == 0 {
                    format!("erro {status}")
                } else {
                    problem.message()
                }))
            }
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        let response = self.send::<()>(Method::GET, path, None).await?;
        Self::parse(response).await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> ApiResult<T> {
        let response = self.send(Method::POST, path, Some(body)).await?;
        Self::parse(response).await
    }

    // -- Autenticação ------------------------------------------------------

    pub async fn register(&self, username: &str, password: &str) -> ApiResult<serde_json::Value> {
        self.post(
            "/auth/register",
            &Credentials {
                username: username.to_owned(),
                password: password.to_owned(),
            },
        )
        .await
    }

    pub async fn login(&self, username: &str, password: &str) -> ApiResult<LoginResponse> {
        self.post(
            "/auth/login",
            &Credentials {
                username: username.to_owned(),
                password: password.to_owned(),
            },
        )
        .await
    }

    pub async fn whoami(&self) -> ApiResult<Whoami> {
        self.get("/auth/whoami").await
    }

    pub async fn logout(&self) -> ApiResult<()> {
        let _ = self.send::<()>(Method::POST, "/auth/logout", None).await;
        self.session.set_token(None);
        Ok(())
    }

    // -- Servidor ----------------------------------------------------------

    /// `None` quando a instância ainda não tem servidor criado.
    pub async fn server(&self) -> ApiResult<Option<Server>> {
        match self.get::<Server>("/server").await {
            Ok(server) => Ok(Some(server)),
            Err(ApiError::NotFound) => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub async fn create_server(&self, name: &str) -> ApiResult<serde_json::Value> {
        self.post(
            "/server",
            &CreateServerRequest {
                name: name.to_owned(),
                password: None,
                public: true,
            },
        )
        .await
    }

    // -- Canais, mensagens e pessoas --------------------------------------

    pub async fn channels(&self) -> ApiResult<Vec<Channel>> {
        let list: ChannelList = self.get("/channels").await?;
        Ok(list.channels)
    }

    pub async fn messages(&self, channel_id: &str) -> ApiResult<MessageList> {
        self.get(&format!("/channels/{channel_id}/messages")).await
    }

    /// O endpoint aceita multipart porque carrega anexos; sem anexo, só os
    /// campos de texto.
    pub async fn send_message(
        &self,
        channel_id: &str,
        content: &str,
        reply_to: Option<&str>,
    ) -> ApiResult<Message> {
        let url = self
            .base
            .join("/messages")
            .map_err(|e| ApiError::Network(e.to_string()))?;

        let mut form = reqwest::multipart::Form::new()
            .text("channel_id", channel_id.to_owned())
            .text("content", content.to_owned());
        if let Some(reply_to) = reply_to {
            form = form.text("reply_to", reply_to.to_owned());
        }

        let mut request = self.http.post(url).multipart(form);
        if let Some(token) = self.session.token() {
            if let Ok(value) = HeaderValue::from_str(&format!("{COOKIE_NAME}={token}")) {
                request = request.header(COOKIE, value);
            }
        }
        let response = request
            .send()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        self.session.absorb(&response);
        Self::parse(response).await
    }

    pub async fn users(&self) -> ApiResult<Vec<UserSummary>> {
        let list: UserList = self.get("/users").await?;
        Ok(list.users)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deriva_o_endereco_do_websocket() {
        let session = Arc::new(Session::default());
        let api = Api::new("https://papo-backend.onrender.com", session).unwrap();
        assert_eq!(api.websocket_url(), "wss://papo-backend.onrender.com/ws");

        let session = Arc::new(Session::default());
        let api = Api::new("http://localhost:8080", session).unwrap();
        assert_eq!(api.websocket_url(), "ws://localhost:8080/ws");
    }
}
