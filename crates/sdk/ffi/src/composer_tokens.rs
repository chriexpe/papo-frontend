//! UTF-16 mention binding/decoding and syntax highlight preview span helpers for native composers.
//! Auto-pair closing, selection formatting, and spoiler filename helpers live in `composer.rs`.

use papo_sdk::Uuid;
use papo_sdk_ui::message_content::{
    ComposerMentionBinding, CustomEmojiRef, PreviewSpan, PreviewSpanKind, decode_composer_mentions,
    encode_composer_mentions, preview_spans_with_bindings_and_emojis, utf16_range_to_utf8,
};

use crate::{
    errors::FfiClientError,
    ids::parse_uuid,
    message_content::{FfiCustomEmojiRef, FfiMentionTarget},
};

/// UTF-16 span bound to a displayed `@label` inside Kotlin composer text state.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiComposerMentionBinding {
    /// Start index in UTF-16 code units.
    pub start_utf16: u32,
    /// Exclusive end index in UTF-16 code units.
    pub end_utf16: u32,
    /// Bound canonical user UUID string.
    pub user_id: String,
    /// Exact display slice (`&text[byte_range]`, typically `@display_name`).
    pub text: String,
}

/// Decoded composer display text paired with UTF-16 mention bindings.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiDecodedComposerMentions {
    /// User-editable display text.
    pub text: String,
    /// Exact UTF-16 mention spans matching `text`.
    pub bindings: Vec<FfiComposerMentionBinding>,
}

/// Preview syntax highlight kind produced by `papo-sdk-ui::message_content::preview`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiPreviewSpanKind {
    /// ATX or Setext heading (`1..=6`).
    Heading {
        /// Heading depth (`1..=6`).
        level: u8,
    },
    /// Bold emphasis.
    Bold,
    /// Italic emphasis.
    Italic,
    /// Strikethrough styling.
    Strikethrough,
    /// Monospace inline code.
    Code,
    /// Monospace fenced or indented code block.
    CodeBlock,
    /// Spoiler hidden syntax span.
    Spoiler,
    /// Safe HTTP/HTTPS/mailto link span.
    Link,
    /// Blockquote styling.
    BlockQuote,
    /// User or `@everyone` mention span.
    Mention {
        /// Resolved mention target.
        target: FfiMentionTarget,
    },
    /// Custom server `:shortcode:` emoji span.
    CustomEmoji {
        /// Custom emoji UUID string.
        id: String,
    },
}

impl From<&PreviewSpanKind> for FfiPreviewSpanKind {
    fn from(value: &PreviewSpanKind) -> Self {
        match value {
            PreviewSpanKind::Heading(level) => Self::Heading { level: *level },
            PreviewSpanKind::Bold => Self::Bold,
            PreviewSpanKind::Italic => Self::Italic,
            PreviewSpanKind::Strikethrough => Self::Strikethrough,
            PreviewSpanKind::Code => Self::Code,
            PreviewSpanKind::CodeBlock => Self::CodeBlock,
            PreviewSpanKind::Spoiler => Self::Spoiler,
            PreviewSpanKind::Link => Self::Link,
            PreviewSpanKind::BlockQuote => Self::BlockQuote,
            PreviewSpanKind::Mention(target) => Self::Mention {
                target: target.into(),
            },
            PreviewSpanKind::CustomEmoji(id) => Self::CustomEmoji { id: id.to_string() },
        }
    }
}

/// One UTF-16 syntax preview highlight span for the native composer.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiPreviewSpan {
    /// Start index in UTF-16 code units.
    pub start_utf16: u32,
    /// Exclusive end index in UTF-16 code units.
    pub end_utf16: u32,
    /// Highlight category.
    pub kind: FfiPreviewSpanKind,
}

/// Pair associating `user_id` with its display label for mention decoding/encoding.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiMentionDisplayName {
    /// User UUID string.
    pub user_id: String,
    /// Display name without leading `@`.
    pub display_name: String,
}

/// Converts validated FFI mention bindings into `ComposerMentionBinding`s using `display_text`.
fn bindings_to_sdk(
    display_text: &str,
    bindings: Vec<FfiComposerMentionBinding>,
) -> Result<Vec<ComposerMentionBinding>, FfiClientError> {
    let mut out = Vec::with_capacity(bindings.len());
    for b in bindings {
        let utf16_range = b.start_utf16 as usize..b.end_utf16 as usize;
        let byte_range = utf16_range_to_utf8(display_text, &utf16_range)
            .ok_or(FfiClientError::InvalidArgument)?;
        out.push(ComposerMentionBinding {
            id: parse_uuid(&b.user_id)?,
            byte_range,
            utf16_range,
            text: b.text,
        });
    }
    Ok(out)
}

/// Decodes canonical backend `<@uuid>` tokens into UTF-16 composer display bindings.
///
/// # Errors
/// Returns [`FfiClientError::InvalidArgument`] or [`FfiClientError::IndexOutOfRange`].
#[uniffi::export]
pub fn decode_composer_mentions_utf16(
    canonical_text: String,
    display_names: Vec<FfiMentionDisplayName>,
) -> Result<FfiDecodedComposerMentions, FfiClientError> {
    let mut parsed_names: Vec<(Uuid, String)> = Vec::with_capacity(display_names.len());
    for entry in display_names {
        parsed_names.push((parse_uuid(&entry.user_id)?, entry.display_name));
    }
    let decoded = decode_composer_mentions(&canonical_text, |id| {
        parsed_names
            .iter()
            .find(|(uid, _)| *uid == id)
            .map(|(_, name)| name.as_str())
    });
    let mut bindings = Vec::with_capacity(decoded.bindings.len());
    for b in decoded.bindings {
        bindings.push(FfiComposerMentionBinding {
            start_utf16: u32::try_from(b.utf16_range.start)
                .map_err(|_| FfiClientError::IndexOutOfRange)?,
            end_utf16: u32::try_from(b.utf16_range.end)
                .map_err(|_| FfiClientError::IndexOutOfRange)?,
            user_id: b.id.to_string(),
            text: b.text,
        });
    }
    Ok(FfiDecodedComposerMentions {
        text: decoded.text,
        bindings,
    })
}

/// Encodes display text and UTF-16 mention bindings back into canonical backend markup.
///
/// # Errors
/// Returns [`FfiClientError::InvalidArgument`] on malformed UUID or out-of-range UTF-16 span.
#[uniffi::export]
pub fn encode_composer_mentions_utf16(
    display_text: String,
    bindings: Vec<FfiComposerMentionBinding>,
) -> Result<String, FfiClientError> {
    let sdk_bindings = bindings_to_sdk(&display_text, bindings)?;
    Ok(encode_composer_mentions(&display_text, &sdk_bindings))
}

/// Computes composer UTF-16 highlight preview spans including bound mentions and custom emojis.
///
/// # Errors
/// Returns [`FfiClientError::InvalidArgument`] or [`FfiClientError::IndexOutOfRange`].
#[uniffi::export]
pub fn composer_preview_spans_utf16(
    display_text: String,
    bindings: Vec<FfiComposerMentionBinding>,
    custom_emojis: Vec<FfiCustomEmojiRef>,
) -> Result<Vec<FfiPreviewSpan>, FfiClientError> {
    let sdk_bindings = bindings_to_sdk(&display_text, bindings)?;
    let catalog: Vec<CustomEmojiRef> = custom_emojis
        .into_iter()
        .map(TryFrom::try_from)
        .collect::<Result<_, _>>()?;
    let spans = preview_spans_with_bindings_and_emojis(&display_text, &sdk_bindings, &catalog);
    spans
        .into_iter()
        .map(
            |PreviewSpan {
                 kind, utf16_range, ..
             }| {
                Ok(FfiPreviewSpan {
                    start_utf16: u32::try_from(utf16_range.start)
                        .map_err(|_| FfiClientError::IndexOutOfRange)?,
                    end_utf16: u32::try_from(utf16_range.end)
                        .map_err(|_| FfiClientError::IndexOutOfRange)?,
                    kind: (&kind).into(),
                })
            },
        )
        .collect()
}
