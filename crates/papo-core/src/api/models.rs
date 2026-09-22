//! Tipos do contrato REST/WebSocket do papo-backend.
//!
//! Os campos acompanham o contrato mesmo antes de a interface usá-los: assim
//! o que chega do servidor fica documentado em um lugar só.
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct Credentials {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginResponse {
    pub user: Option<LoginUser>,
    #[serde(default)]
    pub connection_violation: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LoginUser {
    pub id: String,
    pub username: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Whoami {
    pub id: String,
    pub username: String,
    pub nickname: Option<String>,
    pub status: Option<String>,
    pub status_message: Option<String>,
    #[serde(default)]
    pub roles: Vec<RoleSummary>,
}

impl Whoami {
    pub fn display_name(&self) -> &str {
        self.nickname.as_deref().unwrap_or(&self.username)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoleSummary {
    pub id: String,
    pub name: String,
    /// Cor no formato `#RRGGBB`, quando o cargo define uma.
    pub color: Option<String>,
    #[serde(default)]
    pub position: i32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Server {
    pub id: String,
    pub name: String,
    pub owner_id: Option<String>,
    pub owner_username: Option<String>,
    #[serde(default)]
    pub public: bool,
    #[serde(default)]
    pub member_count: i64,
    #[serde(default)]
    pub channel_count: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateServerRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    pub public: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Channel {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub position: i32,
    pub topic: Option<String>,
    pub last_read_message: Option<String>,
    pub last_message: Option<ChannelLastMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelLastMessage {
    pub id: Option<String>,
    pub content: Option<String>,
    pub author_id: Option<String>,
    pub author_username: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
}

/// Criação de canal (`POST /channels`). O `type` do contrato é uma das três
/// palavras `text`, `voice` ou `category`; o tópico só vale para os dois
/// primeiros, então vai fora quando está vazio.
#[derive(Debug, Clone, Serialize)]
pub struct CreateChannelRequest {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
}

/// Edição de canal (`PUT /channels/{id}`). Tópico ausente não mexe no que
/// está lá; string vazia limpa — é a distinção que o `Option` guarda.
#[derive(Debug, Clone, Serialize)]
pub struct UpdateChannelRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
}

/// Edição do servidor (`PUT /server`). Ícone e senha só vão quando mudam;
/// `public: false` sem senha é recusado pelo backend.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateServerRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_blob: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon_format: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public: Option<bool>,
}

/// Perfil próprio (`PUT /users/{id}`). Os três primeiros são obrigatórios no
/// contrato, mesmo vazios; `typing` ausente não mexe na frase atual.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateUserRequest {
    pub nickname: String,
    pub status: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typing: Option<String>,
}

/// Presença persistida: `away`, `busy` ou nada.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UpdateStatusRequest {
    pub status: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateAvatarRequest {
    pub avatar: String,
    pub avatar_format: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateBannerRequest {
    pub banner: String,
    pub banner_format: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangePasswordRequest {
    pub password: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BanUserRequest {
    pub ban_state: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChangeChannelPositionRequest {
    pub old_position: i32,
    pub new_position: i32,
}

/// `off`, `only_mentions` ou `all`. Sem linha no banco vale `only_mentions`.
#[derive(Debug, Clone, Serialize)]
pub struct ChannelUserSettingRequest {
    pub notification_settings: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DropConnectionRequest {
    /// Uuid da conexão, ou `ALL` para derrubar todas.
    pub connection_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectionInfo {
    pub id: String,
    pub created_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConnectedDevices {
    #[serde(default, deserialize_with = "nullable_list")]
    pub connections: Vec<ConnectionInfo>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateEmojiRequest {
    pub name: String,
    pub image_blob: String,
    pub format: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserProfile {
    pub id: String,
    pub username: String,
    pub nickname: Option<String>,
    pub avatar_blob: Option<String>,
    pub avatar_format: Option<String>,
    pub banner_media: Option<String>,
    pub description: Option<String>,
    pub status: Option<String>,
    #[serde(default)]
    pub roles: Vec<RoleSummary>,
}

/// O campo é `ids`, não `user_ids`, e o backend aceita no máximo 50 por
/// chamada.
#[derive(Debug, Clone, Serialize)]
pub struct ProfileBatchRequest {
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProfileBatchResponse {
    #[serde(default, deserialize_with = "nullable_list")]
    pub profiles: Vec<UserProfile>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuditLogEntry {
    pub id: String,
    #[serde(default)]
    pub actor_username: String,
    #[serde(default)]
    pub action: String,
    #[serde(default)]
    pub entity_type: String,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuditLogList {
    #[serde(default, deserialize_with = "nullable_list")]
    pub logs: Vec<AuditLogEntry>,
    #[serde(default)]
    pub has_more: bool,
}

/// As sete permissões de um cargo, exatamente como o `RolePermissions` do
/// backend. `send_attachment` é a que libera anexo; `manage_channels`, a que
/// libera criar canal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RolePermissions {
    #[serde(default)]
    pub manage_server: bool,
    #[serde(default)]
    pub manage_channels: bool,
    #[serde(default)]
    pub manage_roles: bool,
    #[serde(default)]
    pub ban_members: bool,
    #[serde(default)]
    pub pin_message: bool,
    #[serde(default)]
    pub everyone_message: bool,
    #[serde(default)]
    pub send_attachment: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Role {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
    #[serde(default)]
    pub permissions: RolePermissions,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoleList {
    #[serde(default, deserialize_with = "nullable_list")]
    pub roles: Vec<Role>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoleRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    pub permissions: RolePermissions,
}

#[derive(Debug, Clone, Serialize)]
pub struct AssignRoleRequest {
    pub role_id: String,
}

/// Busca (`POST /search`). Pelo menos um campo tem de vir preenchido; hoje a
/// janela manda só o texto, e os filtros ficam para quando houver tela deles.
#[derive(Debug, Clone, Serialize)]
pub struct SearchRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contains_attachment: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResult {
    #[serde(rename = "type", default)]
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub channel_name: String,
    #[serde(default)]
    pub author_username: String,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResponse {
    #[serde(default, deserialize_with = "nullable_list")]
    pub results: Vec<SearchResult>,
    #[serde(default)]
    pub has_more: bool,
}

/// Um servidor ICE como o backend o entrega (mesmo formato do
/// `RTCIceServer` do navegador). `urls` vem como lista; `username` e
/// `credential` só existem no TURN, e são efêmeros.
#[derive(Debug, Clone, Deserialize)]
pub struct IceServer {
    #[serde(default)]
    pub urls: Vec<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub credential: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct IceServersResponse {
    #[serde(default, deserialize_with = "nullable_list")]
    pub ice_servers: Vec<IceServer>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelList {
    #[serde(default)]
    pub channels: Vec<Channel>,
}

/// Listas que o servidor devolve como `null` quando estão vazias.
fn nullable_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub content: Option<String>,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    /// Id da mensagem respondida. Sem chave estrangeira no banco: pode
    /// apontar para uma mensagem já excluída.
    pub reply_to: Option<String>,
    #[serde(default, deserialize_with = "nullable_list")]
    pub attachments: Vec<Attachment>,
    #[serde(default, deserialize_with = "nullable_list")]
    pub previews: Vec<LinkPreview>,
    /// Contagem por tipo de emoji, sem a lista de quem reagiu.
    #[serde(default, deserialize_with = "nullable_list")]
    pub reactions: Vec<ReactionSummary>,
    /// Com quais emojis o usuário autenticado já reagiu.
    #[serde(default, deserialize_with = "nullable_list")]
    pub user_reactions: Vec<UserReaction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub mime_type: Option<String>,
    pub original_file_name: Option<String>,
    #[serde(default)]
    pub size_bytes: i64,
    /// Miniatura gerada no upload (WebP, ou GIF para GIF animado).
    pub thumbnail_id: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    /// `pending`/`processing`/`clean`/`sensitive`/`blocked`/`failed`.
    pub moderation_status: Option<String>,
}

impl Attachment {
    pub fn name(&self) -> &str {
        self.original_file_name.as_deref().unwrap_or("arquivo")
    }

    pub fn mime(&self) -> &str {
        self.mime_type.as_deref().unwrap_or("application/octet-stream")
    }

    /// O que este anexo é, para a interface escolher como desenhá-lo.
    pub fn kind(&self) -> Kind {
        Kind::guess(self.mime(), self.name())
    }

    /// Conteúdo que a moderação marcou como sensível fica embaçado até um
    /// clique.
    pub fn sensitive(&self) -> bool {
        self.moderation_status.as_deref() == Some("sensitive")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinkPreview {
    pub id: String,
    /// URL normalizada pelo backend.
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub kind: String,
    pub title: Option<String>,
    pub description: Option<String>,
    /// O contrato atual chama este campo de `provider_name`; o alias mantém
    /// compatibilidade com servidores antigos que usavam `site_name`.
    #[serde(default, alias = "site_name")]
    pub provider_name: Option<String>,
    /// Hoje o backend só preenche para embeds allowlistados (YouTube no MVP).
    pub embed_url: Option<String>,
    pub image_mime_type: Option<String>,
    pub image_size_bytes: Option<i64>,
    pub fetched_at: Option<DateTime<Utc>>,
    /// Só vem em GET /link-previews/:id e em link_preview_update; a listagem
    /// de mensagens carrega os metadados sem duplicar a imagem em base64.
    #[serde(default)]
    pub image_data: Option<String>,
}

/// Um emoji de reação: unicode ou emoji custom do servidor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReactionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unicode: Option<String>,
}

impl ReactionRequest {
    pub fn unicode(emoji: impl Into<String>) -> Self {
        Self {
            emoji_id: None,
            unicode: Some(emoji.into()),
        }
    }

    pub fn custom(id: impl Into<String>) -> Self {
        Self {
            emoji_id: Some(id.into()),
            unicode: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReactionSummary {
    pub emoji_id: Option<String>,
    pub unicode: Option<String>,
    #[serde(default)]
    pub count: u32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserReaction {
    pub id: Option<String>,
    pub emoji_id: Option<String>,
    pub unicode: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateMessageRequest {
    pub content: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageList {
    pub channel_id: Option<String>,
    #[serde(default, deserialize_with = "nullable_list")]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PinnedMessage {
    pub message_id: Option<String>,
    pub message: Option<Message>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PinnedList {
    #[serde(default, deserialize_with = "nullable_list")]
    pub pinned: Vec<PinnedMessage>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Emoji {
    pub id: String,
    pub name: String,
    /// `GIF`, `JPEG`, `JPG`, `PNG` ou `WEBP`.
    pub format: Option<String>,
    /// Imagem em base64 — o emoji não tem endpoint próprio de download.
    pub image_blob: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EmojiList {
    #[serde(default, deserialize_with = "nullable_list")]
    pub emojis: Vec<Emoji>,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Notification {
    pub id: String,
    pub message_id: Option<String>,
    pub channel_id: Option<String>,
    pub author_id: Option<String>,
    pub message_content: Option<String>,
    #[serde(default)]
    pub read: bool,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NotificationList {
    #[serde(default, deserialize_with = "nullable_list")]
    pub notifications: Vec<Notification>,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarkNotificationsReadRequest {
    pub notification_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserSummary {
    pub id: String,
    pub username: String,
    pub nickname: Option<String>,
    pub status: Option<String>,
    pub status_message: Option<String>,
    #[serde(default)]
    pub roles: Vec<RoleSummary>,
}

impl UserSummary {
    pub fn display_name(&self) -> &str {
        self.nickname.as_deref().unwrap_or(&self.username)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct UserList {
    #[serde(default, deserialize_with = "nullable_list")]
    pub users: Vec<UserSummary>,
    #[serde(default)]
    pub has_more: bool,
}

/// Erro no formato RFC 7807 devolvido pelo backend.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Problem {
    /// URI do tipo do erro; o final identifica o caso
    /// (`…/errors/server-access-required`, por exemplo).
    #[serde(rename = "type", default)]
    pub kind: String,
    pub title: Option<String>,
    pub detail: Option<String>,
    #[serde(default)]
    pub status: u16,
}

impl Problem {
    pub fn message(&self) -> String {
        match (&self.detail, &self.title) {
            (Some(detail), _) if !detail.is_empty() => detail.clone(),
            (_, Some(title)) if !title.is_empty() => title.clone(),
            _ => format!("erro {}", self.status),
        }
    }
}

/// Converte `#RRGGBB` nos três canais RGB, sem depender da interface.
pub fn parse_hex_color(hex: &str) -> Option<[u8; 3]> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some([
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ])
}

/// Família de mídia derivada do mime type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Image,
    Video,
    Audio,
    Other,
}

impl Kind {
    pub fn of(mime: &str) -> Self {
        match mime.split('/').next().unwrap_or("") {
            "image" => Self::Image,
            "video" => Self::Video,
            "audio" => Self::Audio,
            _ => Self::Other,
        }
    }

    /// Tipo deduzido da extensão do nome do arquivo.
    fn of_extension(name: &str) -> Option<Self> {
        let extension = name.rsplit_once('.')?.1.to_lowercase();
        let kind = match extension.as_str() {
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tif" | "tiff" | "avif" | "heic"
            | "heif" | "svg" | "ico" | "jxl" => Self::Image,
            "mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "ogv" | "wmv" | "flv" | "3gp"
            | "mpg" | "mpeg" | "ts" | "m2ts" => Self::Video,
            "mp3" | "ogg" | "oga" | "opus" | "wav" | "flac" | "m4a" | "aac" | "wma" | "aif"
            | "aiff" | "weba" | "mka" => Self::Audio,
            _ => return None,
        };
        Some(kind)
    }

    /// Mime primeiro; a extensão entra quando ele não diz nada.
    ///
    /// O backend deduz o mime sozinho e erra: um `.mp4` volta como
    /// `application/octet-stream` e um recado de voz como `application/ogg`.
    /// Os dois caem em `Other` pelo mime e apareceriam como um arquivo
    /// qualquer, em vez do vídeo e da forma de onda que são.
    pub fn guess(mime: &str, name: &str) -> Self {
        match Self::of(mime) {
            Self::Other => Self::of_extension(name).unwrap_or(Self::Other),
            known => known,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// As sete permissões vão todas no corpo, inclusive as falsas: o
    /// `RolePermissions` do backend não tem `omitempty`, e mandar só as
    /// verdadeiras deixaria as outras no que já estavam.
    #[test]
    fn cargo_manda_as_sete_permissoes() {
        let body = serde_json::to_string(&RoleRequest {
            name: "membro".to_owned(),
            color: Some("#FF0000".to_owned()),
            permissions: RolePermissions {
                send_attachment: true,
                ..Default::default()
            },
        })
        .unwrap();
        assert_eq!(
            body,
            r##"{"name":"membro","color":"#FF0000","permissions":{"manage_server":false,"manage_channels":false,"manage_roles":false,"ban_members":false,"pin_message":false,"everyone_message":false,"send_attachment":true}}"##
        );
    }


    /// Os nomes dos campos vêm do `createChannelRequest` do backend
    /// (`name`, `type`, `topic`). Errar um deles só apareceria como um 400
    /// em tempo de execução, então o teste prende o formato aqui.
    #[test]
    fn canal_novo_sai_com_os_campos_do_contrato() {
        let body = serde_json::to_string(&CreateChannelRequest {
            name: "geral".to_owned(),
            kind: "text".to_owned(),
            topic: Some("conversa".to_owned()),
        })
        .unwrap();
        assert_eq!(
            body,
            r#"{"name":"geral","type":"text","topic":"conversa"}"#
        );
    }

    /// Categoria não aceita tópico: ele sai do corpo em vez de ir vazio.
    #[test]
    fn canal_sem_topico_omite_o_campo() {
        let body = serde_json::to_string(&CreateChannelRequest {
            name: "avisos".to_owned(),
            kind: "category".to_owned(),
            topic: None,
        })
        .unwrap();
        assert_eq!(body, r#"{"name":"avisos","type":"category"}"#);
    }

    /// Na edição, tópico ausente não mexe no que está lá e string vazia
    /// limpa — a diferença entre `None` e `Some("")` é o contrato inteiro.
    #[test]
    fn editar_canal_distingue_ausente_de_vazio() {
        let intact = serde_json::to_string(&UpdateChannelRequest {
            name: "geral".to_owned(),
            topic: None,
        })
        .unwrap();
        assert_eq!(intact, r#"{"name":"geral"}"#);

        let cleared = serde_json::to_string(&UpdateChannelRequest {
            name: "geral".to_owned(),
            topic: Some(String::new()),
        })
        .unwrap();
        assert_eq!(cleared, r#"{"name":"geral","topic":""}"#);
    }

    /// O backend deduz o mime sozinho e erra: um `.mp4` volta como
    /// `application/octet-stream`. Sem a extensão, vídeo e áudio apareciam
    /// como um arquivo qualquer em vez de tocar na mensagem.
    #[test]
    fn extensao_salva_o_mime_generico() {
        assert_eq!(
            Kind::guess("application/octet-stream", "fJyQbm324ZLaFRfw.mp4"),
            Kind::Video
        );
        assert_eq!(
            Kind::guess("application/ogg", "recado-20260919-212338.ogg"),
            Kind::Audio
        );
    }

    /// Mime bom manda: a extensão só entra quando ele não diz nada.
    #[test]
    fn mime_conhecido_tem_a_palavra_final() {
        assert_eq!(Kind::guess("video/mp4", "coisa.bin"), Kind::Video);
        assert_eq!(Kind::guess("image/png", "sem-extensao"), Kind::Image);
    }

    /// Nem todo octet-stream é mídia; um arquivo continua arquivo.
    #[test]
    fn arquivo_sem_pista_continua_arquivo() {
        assert_eq!(Kind::guess("application/octet-stream", "notas.pdf"), Kind::Other);
        assert_eq!(Kind::guess("application/octet-stream", "sem-ponto"), Kind::Other);
    }

    /// O campo é `ids`. Chamei de `user_ids` na primeira versão e o
    /// servidor respondeu 400 — sem foto de perfil para ninguém, calado.
    #[test]
    fn perfis_em_lote_usam_o_campo_ids() {
        let body = serde_json::to_string(&ProfileBatchRequest {
            ids: vec!["a".to_owned()],
        })
        .unwrap();
        assert_eq!(body, r#"{"ids":["a"]}"#);
    }

    /// A busca manda só o que foi preenchido.
    #[test]
    fn busca_omite_os_filtros_vazios() {
        let body = serde_json::to_string(&SearchRequest {
            text: Some("oi".to_owned()),
            author: None,
            order: Some("desc".to_owned()),
            contains_attachment: None,
        })
        .unwrap();
        assert_eq!(body, r#"{"text":"oi","order":"desc"}"#);
    }
}
