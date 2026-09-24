//! Valores de cache persistente e as operações que a Store produz.
//!
//! Estes tipos são deliberadamente independentes das estruturas de domínio da
//! Store: normalizam só o que precisa sobreviver a um reinício e evitam
//! serializar a Store inteira.

use std::collections::HashSet;

use chrono::{DateTime, Local, Utc};

use crate::state::{Channel, ChannelKind, Emoji, Member, Message, Reaction, Server};

/// Limite de mensagens confirmadas guardadas por canal.
pub const MESSAGE_RETENTION: i64 = 500;
/// Limite de mensagens fixadas guardadas por canal, além das recentes.
pub const PINNED_RETENTION: i64 = 200;
/// Limite duro de intenções de envio ainda não resolvidas por conta/servidor.
pub const OUTGOING_LIMIT: i64 = 500;
/// Limite duro de mensagens já tratadas por conta/servidor no ledger.
pub const NOTIFICATION_LEDGER_LIMIT: i64 = 4096;
/// Limite global de metadados de preview reconstruíveis.
pub const PREVIEW_CACHE_LIMIT: i64 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewCacheState {
    Ready,
    Negative,
    RetryAfter,
}

impl PreviewCacheState {
    pub(crate) fn as_db(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Negative => "negative",
            Self::RetryAfter => "retry_after",
        }
    }

    pub(crate) fn from_db(raw: &str) -> Option<Self> {
        match raw {
            "ready" => Some(Self::Ready),
            "negative" => Some(Self::Negative),
            "retry_after" => Some(Self::RetryAfter),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedPreview {
    pub url_key: String,
    pub source_url: String,
    pub state: PreviewCacheState,
    pub kind: Option<String>,
    pub media_url: Option<String>,
    pub image_url: Option<String>,
    pub embed_url: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub provider_name: Option<String>,
    pub resolved_at: i64,
    pub retry_after: Option<i64>,
    pub failure_class: Option<String>,
    pub last_used_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationDecision {
    Delivered,
    Suppressed,
}

impl NotificationDecision {
    pub(crate) fn as_db(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Suppressed => "suppressed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationLedgerEntry {
    pub owner_user_id: String,
    pub message_id: String,
    pub channel_id: String,
    pub notification_id: Option<String>,
    pub decision: NotificationDecision,
    pub reason: Option<String>,
    pub handled_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimResult {
    New,
    AlreadyHandled,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotificationLedgerStats {
    pub rows: i64,
    pub delivered: i64,
    pub suppressed: i64,
}


#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutgoingState {
    Queued,
    Sending,
    UnknownOutcome,
    FailedPermanent,
}

impl OutgoingState {
    pub(crate) fn as_db(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Sending => "sending",
            Self::UnknownOutcome => "unknown_outcome",
            Self::FailedPermanent => "failed_permanent",
        }
    }

    pub(crate) fn from_db(value: &str) -> Option<Self> {
        match value {
            "queued" => Some(Self::Queued),
            "sending" => Some(Self::Sending),
            "unknown_outcome" => Some(Self::UnknownOutcome),
            "failed_permanent" => Some(Self::FailedPermanent),
            _ => None,
        }
    }

    pub fn may_auto_send(self) -> bool {
        matches!(self, Self::Queued)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedOutgoing {
    pub local_id: String,
    pub owner_user_id: String,
    pub channel_id: String,
    pub content: String,
    pub reply_to: Option<String>,
    pub notify_reply: bool,
    pub created_at: i64,
    pub state: OutgoingState,
    pub attempt_count: u32,
    pub last_attempt_at: Option<i64>,
    pub last_error: Option<String>,
}

/// Identidade local persistente e client-only. Dois hashers com seeds
/// aleatórias independentes produzem 128 bits sem adicionar uma dependência
/// só para UUID; timestamp+contador entram como material extra e preservam
/// unicidade mesmo sob rajadas dentro do mesmo processo. O backend nunca vê
/// este valor e ele não é uma chave de idempotência.
pub fn new_local_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let sequence = COUNTER.fetch_add(1, Ordering::Relaxed);

    fn half(nanos: u128, sequence: u64) -> u64 {
        let state = std::collections::hash_map::RandomState::new();
        let mut hasher = state.build_hasher();
        hasher.write_u128(nanos);
        hasher.write_u64(sequence);
        hasher.finish()
    }

    let high = half(nanos, sequence);
    let low = half(nanos ^ u128::from(sequence), sequence.rotate_left(29));
    format!("local-{high:016x}{low:016x}")
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedServer {
    pub name: String,
    pub description: Option<String>,
    pub owner_user_id: Option<String>,
    pub me_user_id: Option<String>,
    pub me_display_name: Option<String>,
    pub me_username: Option<String>,
    pub updated_at: i64,
}

impl CachedServer {
    pub fn from_store(server: &Server, me_id: &str, me_name: &str, me_username: &str) -> Self {
        Self {
            name: server.name.clone(),
            description: server.description.clone(),
            owner_user_id: (!me_id.is_empty()).then(|| me_id.to_owned()),
            me_user_id: (!me_id.is_empty()).then(|| me_id.to_owned()),
            me_display_name: (!me_name.is_empty()).then(|| me_name.to_owned()),
            me_username: (!me_username.is_empty()).then(|| me_username.to_owned()),
            updated_at: now_millis(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedChannel {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub topic: Option<String>,
    pub position: i32,
    /// Cache de exibição apenas; nunca é autoridade depois de reconectar.
    pub unread: bool,
    pub mentions: u32,
}

impl From<&Channel> for CachedChannel {
    fn from(channel: &Channel) -> Self {
        Self {
            id: channel.id.clone(),
            name: channel.name.clone(),
            kind: match channel.kind {
                ChannelKind::Text => "text",
                ChannelKind::Voice => "voice",
                ChannelKind::Category => "category",
            }
            .to_owned(),
            topic: channel.topic.clone(),
            position: channel.position,
            unread: channel.unread,
            mentions: channel.mentions,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedMember {
    pub id: String,
    pub username: String,
    pub name: String,
    pub role_color: Option<[u8; 3]>,
    pub roles: Vec<String>,
}

impl From<&Member> for CachedMember {
    fn from(member: &Member) -> Self {
        Self {
            id: member.id.clone(),
            username: member.username.clone(),
            name: member.name.clone(),
            role_color: member.role_color,
            roles: member.roles.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CachedAttachment {
    pub id: String,
    pub mime_type: Option<String>,
    pub original_file_name: Option<String>,
    pub size_bytes: i64,
    pub thumbnail_id: Option<String>,
    pub created_at: Option<i64>,
    pub moderation_status: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CachedReaction {
    pub emoji_unicode: Option<String>,
    pub emoji_custom: Option<String>,
    pub count: u32,
    pub mine: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CachedMessage {
    pub id: String,
    pub channel_id: String,
    pub author_id: String,
    pub content: String,
    /// Milissegundos desde a época, em UTC.
    pub created_at: i64,
    pub edited: bool,
    pub reply_to: Option<String>,
    pub pinned: bool,
    pub attachments: Vec<CachedAttachment>,
    pub reactions: Vec<CachedReaction>,
}

impl CachedMessage {
    /// Converte uma mensagem já confirmada pelo servidor. Mensagens pendentes
    /// (ecos locais) não são persistidas nesta fase.
    pub fn from_store(message: &Message) -> Self {
        Self {
            id: message.id.clone(),
            channel_id: message.channel_id.clone(),
            author_id: message.author_id.clone(),
            content: message.content.clone(),
            created_at: message.at.with_timezone(&Utc).timestamp_millis(),
            edited: message.edited,
            reply_to: message.reply_to.clone(),
            pinned: message.pinned,
            attachments: message
                .attachments
                .iter()
                .map(|attachment| CachedAttachment {
                    id: attachment.id.clone(),
                    mime_type: attachment.mime_type.clone(),
                    original_file_name: attachment.original_file_name.clone(),
                    size_bytes: attachment.size_bytes,
                    thumbnail_id: attachment.thumbnail_id.clone(),
                    created_at: attachment
                        .created_at
                        .map(|at| at.timestamp_millis()),
                    moderation_status: attachment.moderation_status.clone(),
                })
                .collect(),
            reactions: message
                .reactions
                .iter()
                .map(|reaction| CachedReaction {
                    emoji_unicode: match &reaction.emoji {
                        Emoji::Unicode(value) => Some(value.clone()),
                        Emoji::Custom(_) => None,
                    },
                    emoji_custom: match &reaction.emoji {
                        Emoji::Custom(id) => Some(id.clone()),
                        Emoji::Unicode(_) => None,
                    },
                    count: reaction.count,
                    mine: reaction.mine,
                })
                .collect(),
        }
    }

    /// Converte diretamente a resposta confirmada do backend para que a
    /// confirmação e a remoção da fila possam ser uma única transação.
    pub fn from_api(message: &crate::api::models::Message) -> Self {
        Self {
            id: message.id.clone(),
            channel_id: message.channel_id.clone(),
            author_id: message.author_id.clone(),
            content: message.content.clone().unwrap_or_default(),
            created_at: message.created_at.timestamp_millis(),
            edited: message.edited_at.is_some(),
            reply_to: message.reply_to.clone(),
            pinned: false,
            attachments: message
                .attachments
                .iter()
                .map(|attachment| CachedAttachment {
                    id: attachment.id.clone(),
                    mime_type: attachment.mime_type.clone(),
                    original_file_name: attachment.original_file_name.clone(),
                    size_bytes: attachment.size_bytes,
                    thumbnail_id: attachment.thumbnail_id.clone(),
                    created_at: attachment.created_at.map(|at| at.timestamp_millis()),
                    moderation_status: attachment.moderation_status.clone(),
                })
                .collect(),
            reactions: message
                .reactions
                .iter()
                .map(|reaction| CachedReaction {
                    emoji_unicode: reaction.unicode.clone(),
                    emoji_custom: reaction.emoji_id.clone(),
                    count: reaction.count,
                    mine: false,
                })
                .collect(),
        }
    }

    /// Reconstrói a projeção da Store. Previews ficam vazias de propósito:
    /// elas pertencem a uma fase posterior e se repovoam na reconciliação.
    pub fn to_store(&self) -> Message {
        Message {
            id: self.id.clone(),
            channel_id: self.channel_id.clone(),
            author_id: self.author_id.clone(),
            content: self.content.clone(),
            at: millis_to_local(self.created_at),
            edited: self.edited,
            reply_to: self.reply_to.clone(),
            attachments: self
                .attachments
                .iter()
                .map(|attachment| crate::api::models::Attachment {
                    id: attachment.id.clone(),
                    mime_type: attachment.mime_type.clone(),
                    original_file_name: attachment.original_file_name.clone(),
                    size_bytes: attachment.size_bytes,
                    thumbnail_id: attachment.thumbnail_id.clone(),
                    created_at: attachment
                        .created_at
                        .and_then(DateTime::from_timestamp_millis),
                    moderation_status: attachment.moderation_status.clone(),
                })
                .collect(),
            previews: Vec::new(),
            reactions: self
                .reactions
                .iter()
                .filter_map(|reaction| {
                    let emoji = Emoji::from_parts(
                        reaction.emoji_unicode.clone(),
                        reaction.emoji_custom.clone(),
                    )?;
                    Some(Reaction {
                        emoji,
                        count: reaction.count,
                        mine: reaction.mine,
                    })
                })
                .collect(),
            pinned: self.pinned,
            pending: false,
        }
    }
}

/// Projeção pronta para hidratar uma Store. Cobre um servidor inteiro.
#[derive(Clone, Debug, Default)]
pub struct CachedServerSnapshot {
    pub owner_user_id: Option<String>,
    pub server: Option<CachedServer>,
    pub channels: Vec<CachedChannel>,
    pub members: Vec<CachedMember>,
    pub messages: Vec<CachedMessage>,
    /// Canais com um snapshot gravado, mesmo vazio.
    pub cached_channels: HashSet<String>,
}

impl CachedServerSnapshot {
    pub fn is_empty(&self) -> bool {
        self.server.is_none()
            && self.channels.is_empty()
            && self.members.is_empty()
            && self.messages.is_empty()
            && self.cached_channels.is_empty()
    }
}

/// Efeito de cache derivado de uma mutação da Store.
#[derive(Clone, Debug)]
pub enum CacheOp {
    UpsertServer(CachedServer),
    SetOwner {
        owner_user_id: String,
        me_name: String,
        me_username: String,
    },
    ReplaceChannels(Vec<CachedChannel>),
    ReplaceMembers(Vec<CachedMember>),
    ReplaceChannelSnapshot {
        channel_id: String,
        messages: Vec<CachedMessage>,
        cached_at: i64,
    },
    UpsertMessage(CachedMessage),
    DeleteMessage { message_id: String },
    /// Snapshot autoritativo de fixadas. Converge também linhas que a Store
    /// não tem carregadas: uma fixada antiga que saiu da janela precisa ser
    /// desafixada para a retenção poder removê-la.
    ///
    /// A ausência desta operação (falha ao buscar pins) preserva o que já
    /// estava gravado.
    ReplacePins {
        channel_id: String,
        ids: Vec<String>,
    },
    /// Apaga somente estado reconstruível do servidor, preservando intenções
    /// de envio particionadas por conta.
    ClearCachedData,
    /// Apaga tudo deste servidor, inclusive intenções de envio (remoção do
    /// servidor/endereço local).
    ClearServer,
}

impl CacheOp {
    /// Operações de posse/controle não podem ser descartadas: um clear perdido
    /// deixaria a conversa da conta anterior no lugar. Dados reconstruíveis
    /// continuam best-effort.
    pub fn is_control(&self) -> bool {
        matches!(
            self,
            CacheOp::ClearServer | CacheOp::ClearCachedData | CacheOp::SetOwner { .. }
        )
    }
}

pub fn now_millis() -> i64 {
    Utc::now().timestamp_millis()
}

fn millis_to_local(ms: i64) -> DateTime<Local> {
    DateTime::from_timestamp_millis(ms)
        .map(|at| at.with_timezone(&Local))
        .unwrap_or_else(Local::now)
}


#[cfg(test)]
mod outgoing_state_tests {
    use super::OutgoingState;

    #[test]
    fn queued_is_the_only_automatic_send_state() {
        assert!(OutgoingState::Queued.may_auto_send());
        assert!(!OutgoingState::Sending.may_auto_send());
        assert!(!OutgoingState::UnknownOutcome.may_auto_send());
        assert!(!OutgoingState::FailedPermanent.may_auto_send());
    }
}
