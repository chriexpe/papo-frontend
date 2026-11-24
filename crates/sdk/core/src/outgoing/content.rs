//! Immutable outgoing message inputs, staged attachment references and draft bindings; persistence and dispatch live elsewhere.

use crate::{AccountId, AttachmentEdit, Uuid};
use papo_api::wire::EmbedInput;
use serde::{Deserialize, Serialize};

/// Largest attachment the SDK stages or sends, in bytes (100 MiB). The backend states no limit, so
/// clients show this one; the backend's own body cap (110 MiB) sits above it.
pub const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;
/// Most bytes all attachments of one message may total (100 MiB).
pub const MAX_MESSAGE_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;
/// Most files one message may carry.
pub const MAX_ATTACHMENTS_PER_MESSAGE: usize = 10;

/// An authoritative remote ID or an earlier operation's eventual message ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageTarget {
    /// An existing backend message.
    Remote(Uuid),
    /// A preceding create operation in this account and channel.
    Local(Uuid),
}

/// Account-scoped reference to an immutable, fsynced attachment blob in outgoing staging.
///
/// Created by [`crate::OutgoingService::stage_attachment`] or attached when loading a committed
/// draft or operation. While a freshly staged handle is alive in memory, background orphan
/// collection will not delete its blob; once dropped without a durable draft or operation
/// reference (or after process restart), the file remains eligible for collection. Once committed
/// in a draft or queued operation, its immutable bytes stay pinned until `Sent`, explicit discard,
/// draft replacement or account removal. Never retains a caller path or handle.
#[derive(Clone)]
pub struct StagedAttachment {
    account: AccountId,
    digest: [u8; 32],
    size_bytes: u64,
    _pin: Option<std::sync::Arc<super::StagedPin>>,
}

impl std::fmt::Debug for StagedAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StagedAttachment")
            .field("account", &self.account)
            .field("digest", &self.digest_hex())
            .field("size_bytes", &self.size_bytes)
            .finish_non_exhaustive()
    }
}

impl PartialEq for StagedAttachment {
    fn eq(&self, other: &Self) -> bool {
        self.account == other.account
            && self.digest == other.digest
            && self.size_bytes == other.size_bytes
    }
}

impl Eq for StagedAttachment {}

impl StagedAttachment {
    pub(super) const fn new(account: AccountId, digest: [u8; 32], size_bytes: u64) -> Self {
        Self {
            account,
            digest,
            size_bytes,
            _pin: None,
        }
    }

    pub(super) fn with_pin(
        account: AccountId,
        digest: [u8; 32],
        size_bytes: u64,
        pin: std::sync::Arc<super::StagedPin>,
    ) -> Self {
        Self {
            account,
            digest,
            size_bytes,
            _pin: Some(pin),
        }
    }

    /// Account whose private staging directory owns this immutable blob.
    #[must_use]
    pub const fn account(&self) -> AccountId {
        self.account
    }

    /// SHA-256 digest of the staged file contents.
    #[must_use]
    pub const fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    /// Lowercase hexadecimal SHA-256 digest naming the blob inside the account's staging directory.
    #[must_use]
    pub fn digest_hex(&self) -> String {
        super::files::digest_hex(&self.digest)
    }

    /// Exact byte length of the staged file.
    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }
}

/// Payload state of an [`OutgoingAttachment`].
///
/// Committed operations and loaded drafts carry [`Self::Staged`] references (or [`Self::Released`]
/// after `Sent`) instead of materializing whole-file byte vectors in memory.
#[derive(Clone, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum OutgoingAttachmentContent {
    /// Caller-supplied in-memory bytes; staged into the account's content-addressed directory
    /// during `save_draft` or `enqueue` and never stored in SQLite or retained in snapshots.
    Bytes(#[serde(with = "encoded")] Vec<u8>),
    /// Immutable staged blob in this account's durable staging directory.
    /// Never serialized into SQLite payloads; durable references live in the `files` column.
    #[serde(skip)]
    Staged(StagedAttachment),
    /// Metadata retained on a confirmed `Sent` receipt after its staged blob was released.
    #[default]
    Released,
}

/// Attachment input or committed metadata. Names are multipart names; embed `attachment_name`
/// references must match them exactly. Never retains a caller path or platform handle.
#[derive(Clone, Serialize, Deserialize)]
pub struct OutgoingAttachment {
    /// Original filename, without a path.
    pub filename: String,
    #[serde(default)]
    content: OutgoingAttachmentContent,
    /// What the sender chose to do to the file at send time; the staged bytes stay the original.
    #[serde(default, skip_serializing_if = "AttachmentEdit::is_identity")]
    edit: AttachmentEdit,
}

impl OutgoingAttachment {
    /// Creates an attachment from caller-supplied in-memory bytes.
    ///
    /// Enqueue and draft saves stage these bytes through the same incremental content-addressed
    /// pipeline used by [`crate::OutgoingService::stage_attachment`].
    #[must_use]
    pub fn from_bytes(filename: impl Into<String>, bytes: Vec<u8>) -> Self {
        Self {
            filename: filename.into(),
            content: OutgoingAttachmentContent::Bytes(bytes),
            edit: AttachmentEdit::default(),
        }
    }

    /// Creates an attachment from an already-staged account-owned blob reference.
    #[must_use]
    pub fn from_staged(filename: impl Into<String>, staged: StagedAttachment) -> Self {
        Self {
            filename: filename.into(),
            content: OutgoingAttachmentContent::Staged(staged),
            edit: AttachmentEdit::default(),
        }
    }

    /// The edit applied when the message is enqueued; the default sends the bytes unchanged.
    #[must_use]
    pub const fn edit(&self) -> &AttachmentEdit {
        &self.edit
    }

    /// Records the edit for this attachment. It travels with the draft and takes effect at
    /// enqueue; until then the staged original is untouched and the edit can change or be undone.
    #[must_use]
    pub const fn with_edit(mut self, edit: AttachmentEdit) -> Self {
        self.edit = edit;
        self
    }

    pub(super) fn take_edit(&mut self) -> AttachmentEdit {
        std::mem::take(&mut self.edit)
    }

    /// Payload state (inline caller bytes, staged blob reference, or released after `Sent`).
    #[must_use]
    pub const fn content(&self) -> &OutgoingAttachmentContent {
        &self.content
    }

    /// Inline caller bytes, when constructed with [`Self::from_bytes`] and not yet committed.
    /// Committed operations and loaded drafts always return `None`.
    #[must_use]
    pub fn inline_bytes(&self) -> Option<&[u8]> {
        match &self.content {
            OutgoingAttachmentContent::Bytes(bytes) => Some(bytes.as_slice()),
            OutgoingAttachmentContent::Staged(_) | OutgoingAttachmentContent::Released => None,
        }
    }

    /// Immutable staged blob reference, when staged or loaded from durable state.
    #[must_use]
    pub const fn staged(&self) -> Option<&StagedAttachment> {
        match &self.content {
            OutgoingAttachmentContent::Staged(staged) => Some(staged),
            OutgoingAttachmentContent::Bytes(_) | OutgoingAttachmentContent::Released => None,
        }
    }

    /// True after a confirmed `Sent` receipt released the staged blob reference.
    #[must_use]
    pub const fn is_released(&self) -> bool {
        matches!(self.content, OutgoingAttachmentContent::Released)
    }

    /// Known byte size of inline or staged payload; zero once released after `Sent`.
    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        match &self.content {
            OutgoingAttachmentContent::Bytes(bytes) => bytes.len() as u64,
            OutgoingAttachmentContent::Staged(staged) => staged.size_bytes,
            OutgoingAttachmentContent::Released => 0,
        }
    }

    pub(super) fn take_content(&mut self) -> OutgoingAttachmentContent {
        std::mem::take(&mut self.content)
    }

    pub(super) fn set_staged(&mut self, staged: StagedAttachment) {
        self.content = OutgoingAttachmentContent::Staged(staged);
    }

    pub(super) fn release(&mut self) {
        self.content = OutgoingAttachmentContent::Released;
    }
}

/// The exact creation payload, independent of any native composer.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct OutgoingMessage {
    /// Canonical backend text, including any caller-bound mention encoding.
    pub content: String,
    /// Existing or locally queued message being replied to.
    pub reply_to: Option<MessageTarget>,
    /// Custom embed inputs; authoritative validation remains on the backend.
    pub embeds: Vec<EmbedInput>,
    /// Immutable multipart files in order.
    pub attachments: Vec<OutgoingAttachment>,
    /// Optional GIF selection bound to this exact durable message; recents change only in Sent commit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gif: Option<crate::Gif>,
}

/// Durable composer state; bindings retain canonical user IDs, never display-name lookup results.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct DraftBody {
    /// Text, reply, embed and file state to restore verbatim.
    pub message: OutgoingMessage,
    /// Bound mention user IDs; clients own editing offsets and native text controls.
    pub mentions: Vec<Uuid>,
}

/// A FULL-committed channel draft. Revisions increase even across deletion/clearing.
#[derive(Clone)]
pub struct Draft {
    /// Optimistic revision used by enqueue and conditional Sent clearing.
    pub revision: u64,
    /// Immutable draft value.
    pub body: DraftBody,
    /// Caller-supplied Unix microseconds, never used to order sends.
    pub updated_at_us: i64,
}

/// One ordered mutation. A local target becomes an explicit persisted dependency.
#[derive(Clone, Serialize, Deserialize)]
pub enum OperationKind {
    /// Send one multipart message.
    Message(OutgoingMessage),
    /// Replace message content/custom embeds; does not alter another composer draft.
    Edit {
        /// Authoritative message or an earlier local create.
        target: MessageTarget,
        /// Replacement canonical text.
        content: String,
        /// Replacement custom embeds, if supplied.
        embeds: Option<Vec<EmbedInput>>,
    },
    /// Delete an existing or locally queued message.
    Delete {
        /// Message whose creation must have completed.
        target: MessageTarget,
    },
    /// Set this account's reaction membership; backend totals remain authoritative.
    Reaction {
        /// Message whose creation must have completed.
        target: MessageTarget,
        /// Custom emoji UUID, mutually exclusive with unicode.
        emoji_id: Option<Uuid>,
        /// Unicode emoji, mutually exclusive with `emoji_id`.
        unicode: Option<String>,
        /// Add when true, remove when false.
        add: bool,
    },
}

impl OutgoingMessage {
    pub(crate) fn validate(&self) -> Result<(), super::OutgoingError> {
        if let Some(gif) = &self.gif {
            gif.validate()
                .map_err(|_| super::OutgoingError::InvalidInput)?;
            if self.content != gif.url {
                return Err(super::OutgoingError::InvalidInput);
            }
        }
        validate_text(&self.content)?;
        validate_attachments(&self.attachments)
    }
}

fn validate_text(content: &str) -> Result<(), super::OutgoingError> {
    if content.chars().count() > 8192 {
        return Err(super::OutgoingError::InvalidInput);
    }
    Ok(())
}

fn validate_attachments(attachments: &[OutgoingAttachment]) -> Result<(), super::OutgoingError> {
    if attachments.len() > MAX_ATTACHMENTS_PER_MESSAGE {
        return Err(super::OutgoingError::InvalidInput);
    }
    let mut total_bytes: u64 = 0;
    for file in attachments {
        if file.filename.is_empty() || file.filename.contains(['/', '\\', '\r', '\n', '\0']) {
            return Err(super::OutgoingError::InvalidInput);
        }
        if file.is_released() {
            return Err(super::OutgoingError::InvalidInput);
        }
        let size = file.size_bytes();
        total_bytes = total_bytes
            .checked_add(size)
            .ok_or(super::OutgoingError::InvalidInput)?;
        if size > MAX_ATTACHMENT_BYTES || total_bytes > MAX_MESSAGE_ATTACHMENT_BYTES {
            return Err(super::OutgoingError::InvalidInput);
        }
    }
    Ok(())
}

fn validate_reaction(
    has_custom_emoji: bool,
    unicode: Option<&str>,
) -> Result<(), super::OutgoingError> {
    if has_custom_emoji == unicode.is_some() {
        return Err(super::OutgoingError::InvalidInput);
    }
    if unicode.is_some_and(|text| text.is_empty() || text.chars().count() > 16) {
        return Err(super::OutgoingError::InvalidInput);
    }
    Ok(())
}

impl OperationKind {
    pub(super) fn target(&self) -> Option<MessageTarget> {
        match self {
            Self::Message(message) => message.reply_to,
            Self::Edit { target, .. } | Self::Delete { target } | Self::Reaction { target, .. } => {
                Some(*target)
            }
        }
    }
    pub(super) fn dependency(&self) -> Option<Uuid> {
        match self.target() {
            Some(MessageTarget::Local(id)) => Some(id),
            _ => None,
        }
    }
    pub(super) fn validate(&self) -> Result<(), super::OutgoingError> {
        match self {
            Self::Message(message) => message.validate(),
            Self::Edit { content, .. } => validate_text(content),
            Self::Delete { .. } => Ok(()),
            Self::Reaction {
                emoji_id, unicode, ..
            } => validate_reaction(emoji_id.is_some(), unicode.as_deref()),
        }
    }
}

mod encoded {
    //! Compact immutable bytes in public DTO serialization; never formats or logs file contents.
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        STANDARD
            .decode(String::deserialize(deserializer)?)
            .map_err(|_| serde::de::Error::custom("invalid attachment bytes"))
    }
}
