//! Attachment handles, embed inputs, target references, and draft payloads for outgoing mutations.
//! Operation state/queue snapshots live in `outgoing_state.rs`; commands in `outgoing.rs`.

use std::sync::Arc;

use papo_sdk::{
    Draft, EmbedInput, EmbedInputAuthor, EmbedInputFieldsItem, EmbedInputFooter, EmbedMediaInput,
    MessageTarget, OutgoingAttachment, OutgoingAttachmentContent, StagedAttachment,
};

use crate::{
    expressions::FfiGif,
    ids::{FfiAccountId, parse_uuid},
    timeline_errors::FfiOutgoingError,
};

/// Opaque handle holding a private staged attachment blob reference alive in account staging.
#[derive(Clone, uniffi::Object)]
pub struct FfiStagedAttachment {
    /// Underlying SDK staged attachment handle with read pin.
    pub(crate) inner: StagedAttachment,
}

#[uniffi::export]
impl FfiStagedAttachment {
    /// Owning account identifier.
    #[must_use]
    pub fn account(&self) -> FfiAccountId {
        FfiAccountId::from_sdk(self.inner.account())
    }

    /// Lowercase SHA-256 hex digest of staged contents.
    #[must_use]
    pub fn digest_hex(&self) -> String {
        self.inner.digest_hex()
    }

    /// Byte size of staged contents.
    #[must_use]
    pub fn size_bytes(&self) -> u64 {
        self.inner.size_bytes()
    }
}

/// Custom embed media source input (URL or uploaded attachment filename).
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmbedMediaInput {
    /// External HTTPS media URL.
    pub url: Option<String>,
    /// Uploaded attachment filename matching an entry in `attachments`.
    pub attachment_name: Option<String>,
    /// Optional MIME type hint.
    pub mime_type: Option<String>,
}

impl From<FfiEmbedMediaInput> for EmbedMediaInput {
    fn from(v: FfiEmbedMediaInput) -> Self {
        Self {
            url: v.url,
            attachment_name: v.attachment_name,
            mime_type: v.mime_type,
        }
    }
}

impl From<&EmbedMediaInput> for FfiEmbedMediaInput {
    fn from(v: &EmbedMediaInput) -> Self {
        Self {
            url: v.url.clone(),
            attachment_name: v.attachment_name.clone(),
            mime_type: v.mime_type.clone(),
        }
    }
}

/// Custom embed author input.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmbedInputAuthor {
    /// Display name.
    pub name: Option<String>,
    /// Link URL.
    pub url: Option<String>,
    /// Icon URL.
    pub icon_url: Option<String>,
}

impl From<FfiEmbedInputAuthor> for EmbedInputAuthor {
    fn from(v: FfiEmbedInputAuthor) -> Self {
        Self {
            name: v.name,
            url: v.url,
            icon_url: v.icon_url,
        }
    }
}

impl From<&EmbedInputAuthor> for FfiEmbedInputAuthor {
    fn from(v: &EmbedInputAuthor) -> Self {
        Self {
            name: v.name.clone(),
            url: v.url.clone(),
            icon_url: v.icon_url.clone(),
        }
    }
}

/// Custom embed field input.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmbedInputField {
    /// Field title.
    pub name: String,
    /// Field text body.
    pub value: String,
    /// Optional inline flag.
    pub inline_layout: Option<bool>,
}

/// Custom embed input sent with messages or edits.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiEmbedInput {
    /// Embed title.
    pub title: Option<String>,
    /// Embed description.
    pub description: Option<String>,
    /// Link URL.
    pub url: Option<String>,
    /// Color hex string.
    pub color: Option<String>,
    /// Site name.
    pub site_name: Option<String>,
    /// Author section.
    pub author: Option<FfiEmbedInputAuthor>,
    /// Footer text.
    pub footer_text: Option<String>,
    /// Thumbnail media input.
    pub thumbnail: Option<FfiEmbedMediaInput>,
    /// Image media input.
    pub image: Option<FfiEmbedMediaInput>,
    /// Video media input.
    pub video: Option<FfiEmbedMediaInput>,
    /// Structured fields.
    pub fields: Vec<FfiEmbedInputField>,
}

impl From<FfiEmbedInput> for EmbedInput {
    fn from(v: FfiEmbedInput) -> Self {
        Self {
            title: v.title,
            description: v.description,
            url: v.url,
            color: v.color,
            site_name: v.site_name,
            author: v.author.map(Into::into),
            footer: v
                .footer_text
                .map(|text| EmbedInputFooter { text: Some(text) }),
            thumbnail: v.thumbnail.map(Into::into),
            image: v.image.map(Into::into),
            video: v.video.map(Into::into),
            fields: (!v.fields.is_empty()).then(|| {
                v.fields
                    .into_iter()
                    .map(|f| EmbedInputFieldsItem {
                        name: f.name,
                        value: f.value,
                        inline: f.inline_layout,
                    })
                    .collect()
            }),
        }
    }
}

impl From<&EmbedInput> for FfiEmbedInput {
    fn from(v: &EmbedInput) -> Self {
        Self {
            title: v.title.clone(),
            description: v.description.clone(),
            url: v.url.clone(),
            color: v.color.clone(),
            site_name: v.site_name.clone(),
            author: v.author.as_ref().map(Into::into),
            footer_text: v.footer.as_ref().and_then(|f| f.text.clone()),
            thumbnail: v.thumbnail.as_ref().map(Into::into),
            image: v.image.as_ref().map(Into::into),
            video: v.video.as_ref().map(Into::into),
            fields: v
                .fields
                .as_deref()
                .unwrap_or_default()
                .iter()
                .map(|f| FfiEmbedInputField {
                    name: f.name.clone(),
                    value: f.value.clone(),
                    inline_layout: f.inline,
                })
                .collect(),
        }
    }
}

/// Target message for replies, edits, deletes, or reactions.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiMessageTarget {
    /// Authoritative remote message UUID.
    Remote {
        /// Server-assigned message UUID string.
        message_id: String,
    },
    /// Earlier durable local create operation UUID.
    Local {
        /// Local operation UUID string.
        operation_id: String,
    },
}

impl FfiMessageTarget {
    /// Parses into `papo_sdk::MessageTarget`.
    ///
    /// # Errors
    /// Returns [`FfiOutgoingError::InvalidInput`] on malformed UUIDs.
    pub(crate) fn into_sdk(self) -> Result<MessageTarget, FfiOutgoingError> {
        let map_id = |raw: &str| parse_uuid(raw).map_err(|_| FfiOutgoingError::InvalidInput);
        match self {
            Self::Remote { message_id } => Ok(MessageTarget::Remote(map_id(&message_id)?)),
            Self::Local { operation_id } => Ok(MessageTarget::Local(map_id(&operation_id)?)),
        }
    }
}

impl From<&MessageTarget> for FfiMessageTarget {
    fn from(v: &MessageTarget) -> Self {
        match v {
            MessageTarget::Remote(id) => Self::Remote {
                message_id: id.to_string(),
            },
            MessageTarget::Local(id) => Self::Local {
                operation_id: id.to_string(),
            },
        }
    }
}

/// Attachment payload supplied to `enqueue` or `save_draft`.
#[derive(Clone, uniffi::Enum)]
pub enum FfiOutgoingAttachmentInput {
    /// Inline small attachment bytes staged automatically by SDK.
    Bytes {
        /// Original filename without directories.
        filename: String,
        /// Payload bytes.
        bytes: Vec<u8>,
    },
    /// Pre-staged content-addressed attachment handle (for large files).
    Staged {
        /// Original filename without directories.
        filename: String,
        /// Staged blob handle returned by `stage_attachment_file`.
        staged: Arc<FfiStagedAttachment>,
    },
}

/// Committed attachment metadata exposed on queued/sent operations and loaded drafts.
#[derive(Clone, uniffi::Record)]
pub struct FfiOutgoingAttachmentView {
    /// Original filename.
    pub filename: String,
    /// Payload size in bytes (0 after `Released`).
    pub size_bytes: u64,
    /// True once confirmed `Sent` released the staged file.
    pub released: bool,
    /// Staged attachment handle when still pinned on disk.
    pub staged: Option<Arc<FfiStagedAttachment>>,
}

impl From<&OutgoingAttachment> for FfiOutgoingAttachmentView {
    fn from(att: &OutgoingAttachment) -> Self {
        Self {
            filename: att.filename.clone(),
            size_bytes: att.size_bytes(),
            released: att.is_released(),
            staged: match att.content() {
                OutgoingAttachmentContent::Staged(s) => {
                    Some(Arc::new(FfiStagedAttachment { inner: s.clone() }))
                }
                _ => None,
            },
        }
    }
}

/// Complete create payload submitted to `enqueue` or saved in a draft.
#[derive(Clone, uniffi::Record)]
pub struct FfiOutgoingMessageInput {
    /// Canonical message text (including `<@uuid>` tokens).
    pub content: String,
    /// Optional target message being replied to.
    pub reply_to: Option<FfiMessageTarget>,
    /// Custom embeds.
    pub embeds: Vec<FfiEmbedInput>,
    /// Ordered multipart attachments.
    pub attachments: Vec<FfiOutgoingAttachmentInput>,
    /// Optional selected GIF metadata (records recent only after `Sent`).
    pub gif: Option<FfiGif>,
}

/// Composer draft body stored atomically in SDK SQLite.
#[derive(Clone, uniffi::Record)]
pub struct FfiDraftBodyInput {
    /// Message contents, embeds, attachments, and reply target.
    pub message: FfiOutgoingMessageInput,
    /// Bound mention user UUIDs.
    pub mention_user_ids: Vec<String>,
}

/// Loaded durable channel draft returned by `load_draft`.
#[derive(Clone, uniffi::Record)]
pub struct FfiDraft {
    /// Optimistic revision number for `enqueue_draft` / `save_draft`.
    pub revision: u64,
    /// Canonical text body.
    pub content: String,
    /// Optional reply target.
    pub reply_to: Option<FfiMessageTarget>,
    /// Custom embeds.
    pub embeds: Vec<FfiEmbedInput>,
    /// Attachment references (pass `staged` back to `save_draft` to preserve staged blobs!).
    pub attachments: Vec<FfiOutgoingAttachmentView>,
    /// Bound GIF if any.
    pub gif: Option<FfiGif>,
    /// Bound mention user UUIDs.
    pub mention_user_ids: Vec<String>,
    /// Caller timestamp recorded when saved.
    pub updated_at_us: i64,
}

impl From<&Draft> for FfiDraft {
    fn from(d: &Draft) -> Self {
        Self {
            revision: d.revision,
            content: d.body.message.content.clone(),
            reply_to: d.body.message.reply_to.as_ref().map(Into::into),
            embeds: d.body.message.embeds.iter().map(Into::into).collect(),
            attachments: d.body.message.attachments.iter().map(Into::into).collect(),
            gif: d.body.message.gif.as_ref().map(Into::into),
            mention_user_ids: d.body.mentions.iter().map(ToString::to_string).collect(),
            updated_at_us: d.updated_at_us,
        }
    }
}
