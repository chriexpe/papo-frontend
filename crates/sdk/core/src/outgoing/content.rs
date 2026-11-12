//! Immutable outgoing message inputs and draft bindings; persistence and dispatch policy live elsewhere.

use crate::Uuid;
use papo_api::wire::EmbedInput;
use serde::{Deserialize, Serialize};

/// An authoritative remote ID or an earlier operation's eventual message ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageTarget {
    /// An existing backend message.
    Remote(Uuid),
    /// A preceding create operation in this account and channel.
    Local(Uuid),
}

/// Caller-selected immutable bytes. Enqueue copies them into FULL durable storage.
/// Names are multipart names; embed attachment references must match them exactly.
#[derive(Clone, Serialize, Deserialize)]
pub struct OutgoingAttachment {
    /// Original filename, without a path.
    pub filename: String,
    /// Original encoded file contents, never a borrowed path or platform permission.
    #[serde(with = "encoded")]
    pub bytes: Vec<u8>,
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
        let (text, files) = match self {
            Self::Message(message) => (
                Some(message.content.as_str()),
                message.attachments.as_slice(),
            ),
            Self::Edit { content, .. } => (Some(content.as_str()), &[][..]),
            Self::Reaction {
                emoji_id, unicode, ..
            } => {
                if emoji_id.is_some() == unicode.is_some()
                    || unicode
                        .as_ref()
                        .is_some_and(|s| s.is_empty() || s.chars().count() > 16)
                {
                    return Err(super::OutgoingError::InvalidInput);
                }
                (None, &[][..])
            }
            Self::Delete { .. } => (None, &[][..]),
        };
        if text.is_some_and(|text| text.chars().count() > 8192) || files.len() > 10 {
            return Err(super::OutgoingError::InvalidInput);
        }
        let bytes = files
            .iter()
            .try_fold(0_usize, |size, file| size.checked_add(file.bytes.len()));
        if bytes.is_none_or(|bytes| bytes > 100 * 1024 * 1024)
            || files.iter().any(|file| {
                file.filename.is_empty() || file.filename.contains(['/', '\\', '\r', '\n', '\0'])
            })
        {
            return Err(super::OutgoingError::InvalidInput);
        }
        Ok(())
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
