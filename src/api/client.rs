//! Cliente REST.
//!
//! A sessão vive num cookie `Auth` HttpOnly. Num cliente nativo não há CORS
//! nem armazenamento do navegador: guardamos o cookie num pote próprio e o
//! gravamos em disco para sobreviver ao fechamento da janela.

use std::path::Path;
use std::sync::{Arc, RwLock};

use reqwest::header::{HeaderValue, ACCEPT, COOKIE};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use url::Url;

use super::models::Notification as NotificationItem;
use super::models::*;

const COOKIE_NAME: &str = "Auth";

/// Pedaço lido por vez ao mandar um anexo. Grande o bastante para não
/// picotar a rede, pequeno o bastante para o pico de memória não ter nada a
/// ver com o tamanho do arquivo.
const UPLOAD_CHUNK: usize = 64 * 1024;

/// Identificador único por requisição. O backend registra o valor no log e o
/// devolve no corpo do erro, então um problema relatado pelo usuário dá para
/// achar do outro lado. Não vale a pena um UUID de verdade aqui: o relógio em
/// nanossegundos mais um contador já não repete.
fn request_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("papo-{nanos:x}-{count:x}")
}

/// Traduz o corpo de um erro do backend. O 401 tem dois significados bem
/// diferentes: sessão expirada ou servidor fechado esperando a senha.
fn problem_error(status: StatusCode, body: &str) -> ApiError {
    let problem: Problem = serde_json::from_str(body).unwrap_or_default();
    match status {
        StatusCode::UNAUTHORIZED => {
            if problem.kind.ends_with("server-access-required") {
                ApiError::ServerLocked
            } else {
                ApiError::Unauthorized
            }
        }
        StatusCode::NOT_FOUND => ApiError::NotFound,
        _ => ApiError::Problem(if problem.status == 0 {
            format!("erro {status}")
        } else {
            problem.message()
        }),
    }
}

/// Arquivo escolhido no seletor, à espera de subir junto com a mensagem.
#[derive(Debug, Clone)]
pub struct Upload {
    pub path: std::path::PathBuf,
    pub name: String,
    pub mime: String,
    pub size: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    Problem(String),
    #[error("sessão expirada")]
    Unauthorized,
    /// Servidor fechado: falta a senha do servidor (`/auth/login_server`).
    #[error("este servidor pede uma senha")]
    ServerLocked,
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

#[derive(Debug, Serialize)]
struct ServerPassword {
    server_password: String,
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
        let mut request = self
            .http
            .request(method, url)
            .header("X-Request-ID", request_id())
            .header(ACCEPT, "application/problem+json, application/json");
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

        Err(problem_error(status, &body))
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> ApiResult<T> {
        let response = self.send::<()>(Method::GET, path, None).await?;
        Self::parse(response).await
    }

    async fn post<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> ApiResult<T> {
        let response = self.send(Method::POST, path, Some(body)).await?;
        Self::parse(response).await
    }

    async fn put<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> ApiResult<T> {
        let response = self.send(Method::PUT, path, Some(body)).await?;
        Self::parse(response).await
    }

    /// `DELETE` com corpo opcional — as reações identificam o emoji no corpo.
    async fn delete<B: Serialize>(&self, path: &str, body: Option<&B>) -> ApiResult<()> {
        let response = self.send(Method::DELETE, path, body).await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let body = response.text().await.unwrap_or_default();
        Err(problem_error(status, &body))
    }

    /// Baixa um recurso binário (anexo, miniatura ou mídia) inteiro na
    /// memória. Devolve também o mime type informado pelo servidor.
    pub async fn fetch_bytes(&self, path: &str) -> ApiResult<(Vec<u8>, Option<String>)> {
        let response = self.send::<()>(Method::GET, path, None).await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(problem_error(status, &body));
        }
        let mime = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.split(';').next().unwrap_or(value).trim().to_owned());
        let bytes = response
            .bytes()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        Ok((bytes.to_vec(), mime))
    }

    /// Baixa direto para o disco. O anexo não passa inteiro pela memória: um
    /// vídeo de 100 MB custava 100 MB de RAM só para ser gravado no cache.
    pub async fn fetch_to_file(&self, path: &str, dest: &Path) -> ApiResult<()> {
        use futures_util::StreamExt as _;
        use tokio::io::AsyncWriteExt as _;

        let response = self.send::<()>(Method::GET, path, None).await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(problem_error(status, &body));
        }
        if let Some(parent) = dest.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        // Grava num temporário e só depois renomeia. Um download interrompido
        // no meio não pode virar um arquivo de cache truncado, que ninguém
        // revalida e que devolve mídia quebrada para sempre.
        let temp = dest.with_extension("parcial");
        let mut file = tokio::fs::File::create(&temp)
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| ApiError::Network(e.to_string()))?;
            file.write_all(&chunk)
                .await
                .map_err(|e| ApiError::Network(e.to_string()))?;
        }
        file.flush()
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        drop(file);
        tokio::fs::rename(&temp, dest)
            .await
            .map_err(|e| ApiError::Network(e.to_string()))?;
        Ok(())
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

    /// Senha do servidor: um servidor fechado recusa login e cadastro até
    /// receber esta autorização, que vem no mesmo cookie `Auth` e vale meia
    /// hora — tempo de entrar ou criar a conta.
    pub async fn login_server(&self, password: &str) -> ApiResult<()> {
        let response = self
            .send(
                Method::POST,
                "/auth/login_server",
                Some(&ServerPassword {
                    server_password: password.to_owned(),
                }),
            )
            .await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let body = response.text().await.unwrap_or_default();
        Err(problem_error(status, &body))
    }

    /// Renova a sessão. O backend gira o token a cada chamada: o antigo passa
    /// a ser um token reusado e, se voltar a aparecer, derruba todas as
    /// sessões da conta. Por isso só existe um lugar que chama isto.
    pub async fn refresh(&self) -> ApiResult<()> {
        let response = self.send::<()>(Method::POST, "/auth/refresh", None).await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let body = response.text().await.unwrap_or_default();
        Err(problem_error(status, &body))
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

    /// Cria um canal. Exige a permissão `manage_channels`: sem ela o backend
    /// responde 403 e o motivo chega no corpo, que é o que a janela mostra.
    pub async fn create_channel(
        &self,
        name: &str,
        kind: &str,
        topic: Option<&str>,
    ) -> ApiResult<Channel> {
        self.post(
            "/channels",
            &CreateChannelRequest {
                name: name.to_owned(),
                kind: kind.to_owned(),
                topic: topic.map(str::to_owned),
            },
        )
        .await
    }

    pub async fn update_channel(
        &self,
        channel_id: &str,
        name: &str,
        topic: Option<&str>,
    ) -> ApiResult<Channel> {
        self.put(
            &format!("/channels/{channel_id}"),
            &UpdateChannelRequest {
                name: name.to_owned(),
                topic: topic.map(str::to_owned),
            },
        )
        .await
    }

    pub async fn delete_channel(&self, channel_id: &str) -> ApiResult<()> {
        self.delete::<()>(&format!("/channels/{channel_id}"), None)
            .await
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
        attachments: &[Upload],
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
        for upload in attachments {
            // Ler o arquivo inteiro para a memória fazia o pico acompanhar o
            // tamanho do anexo, um para um. Agora o corpo sai em pedaços,
            // direto do disco, e mandar um vídeo grande custa o mesmo que
            // mandar um pequeno.
            let file = tokio::fs::File::open(&upload.path)
                .await
                .map_err(|e| ApiError::Network(format!("{}: {e}", upload.name)))?;
            let length = file
                .metadata()
                .await
                .map_err(|e| ApiError::Network(format!("{}: {e}", upload.name)))?
                .len();
            let chunks = futures_util::stream::try_unfold(file, |mut file| async move {
                use tokio::io::AsyncReadExt as _;
                let mut chunk = vec![0u8; UPLOAD_CHUNK];
                let read = file.read(&mut chunk).await?;
                if read == 0 {
                    return Ok::<_, std::io::Error>(None);
                }
                chunk.truncate(read);
                Ok(Some((chunk, file)))
            });
            // Com o tamanho declarado o servidor recebe um multipart comum,
            // sem codificação em pedaços.
            let part = reqwest::multipart::Part::stream_with_length(
                reqwest::Body::wrap_stream(chunks),
                length,
            )
            .file_name(upload.name.clone())
            .mime_str(&upload.mime)
            .map_err(|e| ApiError::Network(e.to_string()))?;
            form = form.part("attachments", part);
        }

        let mut request = self
            .http
            .post(url)
            .header("X-Request-ID", request_id())
            .header(ACCEPT, "application/problem+json, application/json")
            .multipart(form);
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

    pub async fn edit_message(&self, message_id: &str, content: &str) -> ApiResult<Message> {
        self.put(
            &format!("/messages/{message_id}"),
            &UpdateMessageRequest {
                content: content.to_owned(),
            },
        )
        .await
    }

    pub async fn delete_message(&self, message_id: &str) -> ApiResult<()> {
        self.delete::<()>(&format!("/messages/{message_id}"), None)
            .await
    }

    /// Busca mensagens. O backend limita a 100 por chamada e só devolve o
    /// que o usuário pode ler, então não há o que filtrar deste lado.
    pub async fn search(&self, text: &str) -> ApiResult<SearchResponse> {
        self.post(
            "/search",
            &SearchRequest {
                text: Some(text.to_owned()),
                author: None,
                order: Some("desc".to_owned()),
                contains_attachment: None,
            },
        )
        .await
    }

    // -- Servidor, perfil e conta -----------------------------------------

    /// Edita o servidor. Deixar `public: Some(false)` sem senha é recusado.
    pub async fn update_server(&self, request: &UpdateServerRequest) -> ApiResult<Server> {
        self.put("/server", request).await
    }

    pub async fn update_profile(
        &self,
        user_id: &str,
        request: &UpdateUserRequest,
    ) -> ApiResult<serde_json::Value> {
        self.put(&format!("/users/{user_id}"), request).await
    }

    /// `away`, `busy`, ou `None` para limpar.
    pub async fn set_status(
        &self,
        user_id: &str,
        status: Option<&str>,
    ) -> ApiResult<serde_json::Value> {
        self.put(
            &format!("/users/{user_id}/status"),
            &UpdateStatusRequest {
                status: status.map(str::to_owned),
            },
        )
        .await
    }

    pub async fn set_avatar(
        &self,
        user_id: &str,
        avatar: &str,
        format: &str,
    ) -> ApiResult<serde_json::Value> {
        self.put(
            &format!("/users/{user_id}/avatar"),
            &UpdateAvatarRequest {
                avatar: avatar.to_owned(),
                avatar_format: format.to_owned(),
            },
        )
        .await
    }

    // As cinco daqui até o fim do bloco existem porque o contrato tem, e
    // conferem contra ele; o que falta é tela. `set_banner` e `media` são o
    // banner do perfil, `profile` é a ficha de uma pessoa só,
    // `put_user_settings` guardaria os ajustes no servidor em vez de só em
    // disco, e `link_preview` é a prévia de link que já vem em `previews`.
    #[allow(dead_code)]
    pub async fn set_banner(
        &self,
        user_id: &str,
        banner: &str,
        format: &str,
    ) -> ApiResult<serde_json::Value> {
        self.put(
            &format!("/users/{user_id}/banner"),
            &UpdateBannerRequest {
                banner: banner.to_owned(),
                banner_format: format.to_owned(),
            },
        )
        .await
    }

    #[allow(dead_code)]
    pub async fn profile(&self, user_id: &str) -> ApiResult<UserProfile> {
        self.get(&format!("/users/{user_id}/profile")).await
    }

    /// Perfis em lote: é assim que se busca o avatar de todo mundo sem uma
    /// requisição por pessoa.
    pub async fn profiles(&self, user_ids: Vec<String>) -> ApiResult<Vec<UserProfile>> {
        // O backend recusa mais de 50 de uma vez; um servidor com mais gente
        // que isso receberia um 400 e ninguém teria foto.
        let mut all = Vec::with_capacity(user_ids.len());
        for slice in user_ids.chunks(50) {
            let list: ProfileBatchResponse = self
                .post(
                    "/users/profile_batch",
                    &ProfileBatchRequest {
                        ids: slice.to_vec(),
                    },
                )
                .await?;
            all.extend(list.profiles);
        }
        Ok(all)
    }

    pub async fn change_password(
        &self,
        user_id: &str,
        password: &str,
    ) -> ApiResult<serde_json::Value> {
        self.put(
            &format!("/users/{user_id}/password"),
            &ChangePasswordRequest {
                password: password.to_owned(),
            },
        )
        .await
    }

    pub async fn ban_user(&self, user_id: &str, banned: bool) -> ApiResult<serde_json::Value> {
        self.put(
            &format!("/users/{user_id}/ban"),
            &BanUserRequest { ban_state: banned },
        )
        .await
    }

    pub async fn reset_user(&self, user_id: &str) -> ApiResult<serde_json::Value> {
        self.post(&format!("/users/{user_id}/reset"), &()).await
    }

    /// Ajustes do usuário guardados no servidor, para acompanhá-lo entre
    /// máquinas. O corpo é o `config` inteiro.
    #[allow(dead_code)]
    pub async fn put_user_settings(
        &self,
        config: &serde_json::Value,
    ) -> ApiResult<serde_json::Value> {
        self.put("/users/settings", config).await
    }

    // -- Sessões -----------------------------------------------------------

    pub async fn connected_devices(&self) -> ApiResult<Vec<ConnectionInfo>> {
        let list: ConnectedDevices = self.get("/auth/connected_devices").await?;
        Ok(list.connections)
    }

    /// `ALL` derruba todas as conexões da conta.
    pub async fn drop_connection(&self, connection_id: &str) -> ApiResult<serde_json::Value> {
        self.post(
            "/auth/drop_connection",
            &DropConnectionRequest {
                connection_id: connection_id.to_owned(),
            },
        )
        .await
    }

    // -- Canais: ordem, permissões e notificação --------------------------

    pub async fn move_channel(
        &self,
        channel_id: &str,
        old_position: i32,
        new_position: i32,
    ) -> ApiResult<serde_json::Value> {
        self.put(
            &format!("/channels/{channel_id}/change_position"),
            &ChangeChannelPositionRequest {
                old_position,
                new_position,
            },
        )
        .await
    }

    pub async fn set_channel_notifications(
        &self,
        channel_id: &str,
        user_id: &str,
        setting: &str,
    ) -> ApiResult<serde_json::Value> {
        self.post(
            &format!("/channels/{channel_id}/user/{user_id}/settings"),
            &ChannelUserSettingRequest {
                notification_settings: setting.to_owned(),
            },
        )
        .await
    }

    // -- Emojis do servidor ------------------------------------------------

    pub async fn create_emoji(
        &self,
        name: &str,
        image_blob: &str,
        format: &str,
    ) -> ApiResult<serde_json::Value> {
        self.post(
            "/emojis",
            &CreateEmojiRequest {
                name: name.to_owned(),
                image_blob: image_blob.to_owned(),
                format: format.to_owned(),
            },
        )
        .await
    }

    pub async fn delete_emoji(&self, emoji_id: &str) -> ApiResult<()> {
        self.delete::<()>(&format!("/emojis/{emoji_id}"), None).await
    }

    // -- Mídia por conteúdo ------------------------------------------------

    /// Mídia endereçada pelo sha256 — é como o banner de perfil é servido.
    #[allow(dead_code)]
    pub async fn media(&self, sha_hash: &str) -> ApiResult<(Vec<u8>, Option<String>)> {
        self.fetch_bytes(&format!("/media/{sha_hash}")).await
    }

    #[allow(dead_code)]
    pub async fn link_preview(&self, preview_id: &str) -> ApiResult<serde_json::Value> {
        self.get(&format!("/link-previews/{preview_id}")).await
    }

    // -- Auditoria ---------------------------------------------------------

    pub async fn audit_logs(&self) -> ApiResult<Vec<AuditLogEntry>> {
        let list: AuditLogList = self.get("/admin/audit-logs").await?;
        Ok(list.logs)
    }

    // -- Cargos ------------------------------------------------------------

    pub async fn roles(&self) -> ApiResult<Vec<Role>> {
        let list: RoleList = self.get("/roles").await?;
        Ok(list.roles)
    }

    /// Cria um cargo. Exige `manage_roles`.
    pub async fn create_role(
        &self,
        name: &str,
        color: Option<&str>,
        permissions: RolePermissions,
    ) -> ApiResult<Role> {
        self.post(
            "/roles",
            &RoleRequest {
                name: name.to_owned(),
                color: color.map(str::to_owned),
                permissions,
            },
        )
        .await
    }

    pub async fn update_role(
        &self,
        role_id: &str,
        name: &str,
        color: Option<&str>,
        permissions: RolePermissions,
    ) -> ApiResult<Role> {
        self.put(
            &format!("/roles/{role_id}"),
            &RoleRequest {
                name: name.to_owned(),
                color: color.map(str::to_owned),
                permissions,
            },
        )
        .await
    }

    pub async fn delete_role(&self, role_id: &str) -> ApiResult<()> {
        self.delete::<()>(&format!("/roles/{role_id}"), None).await
    }

    pub async fn assign_role(&self, user_id: &str, role_id: &str) -> ApiResult<serde_json::Value> {
        self.post(
            &format!("/users/{user_id}/roles"),
            &AssignRoleRequest {
                role_id: role_id.to_owned(),
            },
        )
        .await
    }

    pub async fn unassign_role(&self, user_id: &str, role_id: &str) -> ApiResult<()> {
        self.delete::<()>(&format!("/users/{user_id}/roles/{role_id}"), None)
            .await
    }

    // -- Reações -----------------------------------------------------------

    pub async fn add_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &ReactionRequest,
    ) -> ApiResult<serde_json::Value> {
        self.post(
            &format!("/channels/{channel_id}/messages/{message_id}/reactions"),
            emoji,
        )
        .await
    }

    pub async fn remove_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &ReactionRequest,
    ) -> ApiResult<()> {
        self.delete(
            &format!("/channels/{channel_id}/messages/{message_id}/reactions"),
            Some(emoji),
        )
        .await
    }

    /// Emojis custom do servidor, em páginas de 25.
    pub async fn emojis(&self) -> ApiResult<Vec<Emoji>> {
        let mut all = Vec::new();
        let mut page: EmojiList = self.get("/emojis").await?;
        all.append(&mut page.emojis);
        // Uma página costuma bastar; o cursor evita perder emojis num
        // servidor com muitos.
        let mut guard = 0;
        while page.has_more && guard < 20 {
            guard += 1;
            let Some(last) = all.last() else { break };
            let path = format!("/emojis?last_id={}", last.id);
            page = self.get(&path).await?;
            if page.emojis.is_empty() {
                break;
            }
            all.append(&mut page.emojis);
        }
        Ok(all)
    }

    // -- Fixadas -----------------------------------------------------------

    pub async fn pin_message(&self, channel_id: &str, message_id: &str) -> ApiResult<serde_json::Value> {
        self.post(
            &format!("/channels/{channel_id}/messages/{message_id}/pin"),
            &(),
        )
        .await
    }

    pub async fn unpin_message(&self, channel_id: &str, message_id: &str) -> ApiResult<()> {
        self.delete::<()>(
            &format!("/channels/{channel_id}/messages/{message_id}/pin"),
            None,
        )
        .await
    }

    pub async fn pinned(&self, channel_id: &str) -> ApiResult<PinnedList> {
        self.get(&format!("/channels/{channel_id}/pinned")).await
    }

    // -- Notificações ------------------------------------------------------

    /// Notificações do usuário — é daqui que sai a contagem de menções.
    pub async fn notifications(&self, user_id: &str) -> ApiResult<Vec<NotificationItem>> {
        let list: NotificationList = self.get(&format!("/users/{user_id}/notifications")).await?;
        Ok(list.notifications)
    }

    pub async fn mark_notifications_read(
        &self,
        user_id: &str,
        ids: Vec<String>,
    ) -> ApiResult<serde_json::Value> {
        self.put(
            &format!("/users/{user_id}/read_notification"),
            &MarkNotificationsReadRequest {
                notification_ids: ids,
            },
        )
        .await
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
