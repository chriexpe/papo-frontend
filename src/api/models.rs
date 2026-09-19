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
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelList {
    #[serde(default)]
    pub channels: Vec<Channel>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Message {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub content: Option<String>,
    pub created_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub reply_to: Option<String>,
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    #[serde(default)]
    pub size: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Reaction {
    pub emoji: String,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub me: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MessageList {
    pub channel_id: Option<String>,
    #[serde(default)]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub has_more: bool,
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
    #[serde(default)]
    pub users: Vec<UserSummary>,
    #[serde(default)]
    pub has_more: bool,
}

/// Erro no formato RFC 7807 devolvido pelo backend.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Problem {
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
