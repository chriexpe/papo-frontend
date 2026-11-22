//! Authoritative wire `Message`, attachments, embeds, external authors, and reactions for FFI.
//! Preserves timestamp spellings verbatim without synthesizing identities or grouped UI state.

use papo_sdk::{
    Attachment, Embed, EmbedAuthor, EmbedField, EmbedFooter, EmbedMedia, ExternalAuthor, Message,
    MessageReactionSummary, MessageUserReaction,
};

/// External webhook author metadata distinct from registered bridge users.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiExternalAuthor {
    /// Origin platform or bridge source.
    pub source: Option<String>,
    /// External platform identifier.
    pub id: Option<String>,
    /// Display name from webhook payload.
    pub name: Option<String>,
    /// External avatar URL.
    pub avatar_url: Option<String>,
    /// External profile URL.
    pub profile_url: Option<String>,
}

impl From<&ExternalAuthor> for FfiExternalAuthor {
    fn from(value: &ExternalAuthor) -> Self {
        Self {
            source: value.source.clone(),
            id: value.id.clone(),
            name: value.name.clone(),
            avatar_url: value.avatar_url.clone(),
            profile_url: value.profile_url.clone(),
        }
    }
}

/// Authoritative message attachment record.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiAttachment {
    /// Attachment UUID string.
    pub id: Option<String>,
    /// Declared MIME type.
    pub mime_type: Option<String>,
    /// Original filename (including optional spoiler filename prefix).
    pub original_file_name: Option<String>,
    /// Declared byte length.
    pub size_bytes: Option<i64>,
    /// Image/video width in pixels.
    pub width: Option<i64>,
    /// Image/video height in pixels.
    pub height: Option<i64>,
    /// Optional thumbnail media UUID.
    pub thumbnail_id: Option<String>,
    /// Thumbnail width in pixels.
    pub thumbnail_width: Option<i64>,
    /// Thumbnail height in pixels.
    pub thumbnail_height: Option<i64>,
    /// Creation timestamp string.
    pub created_at: Option<String>,
    /// Backend moderation status tag.
    pub moderation_status: Option<String>,
}

impl From<&Attachment> for FfiAttachment {
    fn from(value: &Attachment) -> Self {
        Self {
            id: value.id.clone(),
            mime_type: value.mime_type.clone(),
            original_file_name: value.original_file_name.clone(),
            size_bytes: value.size_bytes,
            width: value.width,
            height: value.height,
            thumbnail_id: value.thumbnail_id.clone(),
            thumbnail_width: value.thumbnail_width,
            thumbnail_height: value.thumbnail_height,
            created_at: value.created_at.clone(),
            moderation_status: value.moderation_status.clone(),
        }
    }
}

/// Rich embed media slot (thumbnail, image, or video).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmbedMedia {
    /// Media URL.
    pub url: Option<String>,
    /// Declared MIME type.
    pub mime_type: Option<String>,
    /// Media width in pixels.
    pub width: Option<i64>,
    /// Media height in pixels.
    pub height: Option<i64>,
}

impl From<&EmbedMedia> for FfiEmbedMedia {
    fn from(value: &EmbedMedia) -> Self {
        Self {
            url: value.url.clone(),
            mime_type: value.mime_type.clone(),
            width: value.width,
            height: value.height,
        }
    }
}

/// Rich embed author metadata.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmbedAuthor {
    /// Author name.
    pub name: Option<String>,
    /// Author link URL.
    pub url: Option<String>,
    /// Author icon media.
    pub media: Option<FfiEmbedMedia>,
}

impl From<&EmbedAuthor> for FfiEmbedAuthor {
    fn from(value: &EmbedAuthor) -> Self {
        Self {
            name: value.name.clone(),
            url: value.url.clone(),
            media: value.media.as_ref().map(Into::into),
        }
    }
}

/// Rich embed footer metadata.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmbedFooter {
    /// Footer text.
    pub text: Option<String>,
    /// Footer icon media.
    pub icon: Option<FfiEmbedMedia>,
}

impl From<&EmbedFooter> for FfiEmbedFooter {
    fn from(value: &EmbedFooter) -> Self {
        Self {
            text: value.text.clone(),
            icon: value.icon.as_ref().map(Into::into),
        }
    }
}

/// Rich embed field entry.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmbedField {
    /// Optional explicit field ordering position.
    pub position: Option<i64>,
    /// Field title.
    pub name: Option<String>,
    /// Field body text.
    pub value: Option<String>,
    /// Inline layout hint.
    pub inline_layout: Option<bool>,
}

impl From<&EmbedField> for FfiEmbedField {
    fn from(value: &EmbedField) -> Self {
        Self {
            position: value.position,
            name: value.name.clone(),
            value: value.value.clone(),
            inline_layout: value.inline,
        }
    }
}

/// Authoritative rich embed attached to a message.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmbed {
    /// Embed UUID string.
    pub id: Option<String>,
    /// Origin type (`custom`, `link`, etc.).
    pub source_type: Option<String>,
    /// Fetch strategy metadata.
    pub fetch_method: Option<String>,
    /// Provider label.
    pub provider: Option<String>,
    /// Site name.
    pub site_name: Option<String>,
    /// Target link URL.
    pub url: Option<String>,
    /// Embed title.
    pub title: Option<String>,
    /// Embed description.
    pub description: Option<String>,
    /// Optional accent hex color string.
    pub color: Option<String>,
    /// Author section.
    pub author: Option<FfiEmbedAuthor>,
    /// Thumbnail media.
    pub thumbnail: Option<FfiEmbedMedia>,
    /// Primary image media.
    pub image: Option<FfiEmbedMedia>,
    /// Primary video media.
    pub video: Option<FfiEmbedMedia>,
    /// Footer section.
    pub footer: Option<FfiEmbedFooter>,
    /// Structured fields.
    pub fields: Vec<FfiEmbedField>,
    /// Canonical embed URL when fetched separately.
    pub embed_url: Option<String>,
    /// Creation timestamp string.
    pub created_at: Option<String>,
    /// Metadata fetch timestamp string.
    pub fetched_at: Option<String>,
}

impl From<&Embed> for FfiEmbed {
    fn from(value: &Embed) -> Self {
        Self {
            id: value.id.clone(),
            source_type: value.source_type.clone(),
            fetch_method: value.fetch_method.clone(),
            provider: value.provider.clone(),
            site_name: value.site_name.clone(),
            url: value.url.clone(),
            title: value.title.clone(),
            description: value.description.clone(),
            color: value.color.clone(),
            author: value.author.as_ref().map(Into::into),
            thumbnail: value.thumbnail.as_ref().map(Into::into),
            image: value.image.as_ref().map(Into::into),
            video: value.video.as_ref().map(Into::into),
            footer: value.footer.as_ref().map(Into::into),
            fields: value
                .fields
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(Into::into)
                .collect(),
            embed_url: value.embed_url.clone(),
            created_at: value.created_at.clone(),
            fetched_at: value.fetched_at.clone(),
        }
    }
}

/// Reaction count aggregate for one emoji on a message.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiMessageReactionSummary {
    /// Custom server emoji UUID if custom.
    pub emoji_id: Option<String>,
    /// Unicode emoji sequence if unicode.
    pub unicode: Option<String>,
    /// Total count reported by backend.
    pub count: Option<i64>,
}

impl From<&MessageReactionSummary> for FfiMessageReactionSummary {
    fn from(value: &MessageReactionSummary) -> Self {
        Self {
            emoji_id: value.emoji_id.clone(),
            unicode: value.unicode.clone(),
            count: value.count,
        }
    }
}

/// Signed-in user's own reaction entry on a message.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiMessageUserReaction {
    /// Reaction entry ID if supplied.
    pub id: Option<String>,
    /// Custom server emoji UUID.
    pub emoji_id: Option<String>,
    /// Unicode emoji sequence.
    pub unicode: Option<String>,
}

impl From<&MessageUserReaction> for FfiMessageUserReaction {
    fn from(value: &MessageUserReaction) -> Self {
        Self {
            id: value.id.clone(),
            emoji_id: value.emoji_id.clone(),
            unicode: value.unicode.clone(),
        }
    }
}

/// Authoritative backend message record mapped faithfully across FFI.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiMessage {
    /// Backend message UUID.
    pub id: Option<String>,
    /// Owning channel or DM UUID.
    pub channel_id: Option<String>,
    /// Author user UUID if present.
    pub author_id: Option<String>,
    /// Canonical message body text.
    pub content: Option<String>,
    /// Exact backend RFC3339 creation timestamp.
    pub created_at: Option<String>,
    /// Exact backend RFC3339 edit timestamp.
    pub edited_at: Option<String>,
    /// Parent message UUID if this message is a reply.
    pub reply_to: Option<String>,
    /// Webhook UUID if posted via webhook.
    pub webhook_id: Option<String>,
    /// Bridged external author metadata if present.
    pub external_author: Option<FfiExternalAuthor>,
    /// Message file attachments.
    pub attachments: Vec<FfiAttachment>,
    /// Rich embeds.
    pub embeds: Vec<FfiEmbed>,
    /// Reaction count summaries.
    pub reactions: Vec<FfiMessageReactionSummary>,
    /// Signed-in user's active reactions.
    pub user_reactions: Vec<FfiMessageUserReaction>,
}

impl From<&Message> for FfiMessage {
    fn from(value: &Message) -> Self {
        Self {
            id: value.id.clone(),
            channel_id: value.channel_id.clone(),
            author_id: value.author_id.clone(),
            content: value.content.clone(),
            created_at: value.created_at.clone(),
            edited_at: value.edited_at.clone(),
            reply_to: value.reply_to.clone(),
            webhook_id: value.webhook_id.clone(),
            external_author: value.external_author.as_ref().map(Into::into),
            attachments: value
                .attachments
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(Into::into)
                .collect(),
            embeds: value
                .embeds
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(Into::into)
                .collect(),
            reactions: value
                .reactions
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(Into::into)
                .collect(),
            user_reactions: value
                .user_reactions
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(Into::into)
                .collect(),
        }
    }
}
