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

    /// Conteúdo que a moderação marcou como sensível fica embaçado até um
    /// clique.
    pub fn sensitive(&self) -> bool {
        self.moderation_status.as_deref() == Some("sensitive")
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LinkPreview {
    pub id: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_name: Option<String>,
    /// Imagem embutida em base64 quando o preview chega por evento.
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

/// Converte `#RRGGBB` em cor do egui.
pub fn parse_hex_color(hex: &str) -> Option<egui::Color32> {
    let hex = hex.trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    Some(egui::Color32::from_rgb(
        (value >> 16) as u8,
        (value >> 8) as u8,
        value as u8,
    ))
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
