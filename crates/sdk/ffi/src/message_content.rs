//! Parsed message AST types and pure content validation across FFI.
//! Native interactive composer editing helpers live in `composer.rs`.

use papo_sdk_ui::message_content::{
    Block, CustomEmojiRef, Inline, ListItem, MessageContent, TableAlignment, TextStyle, is_plain,
    is_safe_href,
};

use crate::{errors::FfiClientError, ids::parse_uuid};
use papo_sdk::mentions::MentionTarget;

/// Cumulative emphasis style active on inline text or mention runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiTextStyle {
    /// Bold emphasis (`**` / `__`).
    pub bold: bool,
    /// Italic emphasis (`*` / `_`).
    pub italic: bool,
    /// Strikethrough (`~` / `~~`).
    pub strikethrough: bool,
}

impl From<TextStyle> for FfiTextStyle {
    fn from(value: TextStyle) -> Self {
        Self {
            bold: value.bold,
            italic: value.italic,
            strikethrough: value.strikethrough,
        }
    }
}

/// GFM table column alignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiTableAlignment {
    /// Left aligned or default.
    Left,
    /// Center aligned.
    Center,
    /// Right aligned.
    Right,
}

impl From<TableAlignment> for FfiTableAlignment {
    fn from(value: TableAlignment) -> Self {
        match value {
            TableAlignment::Left => Self::Left,
            TableAlignment::Center => Self::Center,
            TableAlignment::Right => Self::Right,
        }
    }
}

/// Target of a canonical parsed mention.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiMentionTarget {
    /// Specific registered backend user UUID.
    User {
        /// User UUID string.
        user_id: String,
    },
    /// Channel-wide `@everyone` token.
    Everyone,
}

impl From<&MentionTarget> for FfiMentionTarget {
    fn from(value: &MentionTarget) -> Self {
        match value {
            MentionTarget::User(id) => Self::User {
                user_id: id.to_string(),
            },
            MentionTarget::Everyone => Self::Everyone,
        }
    }
}

/// Minimal caller-supplied custom emoji reference passed into parser and composer preview helpers.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiCustomEmojiRef {
    /// Backend custom emoji UUID string.
    pub id: String,
    /// Shortcode name without colons.
    pub name: String,
}

impl TryFrom<FfiCustomEmojiRef> for CustomEmojiRef {
    type Error = FfiClientError;

    fn try_from(value: FfiCustomEmojiRef) -> Result<Self, Self::Error> {
        Ok(Self {
            id: parse_uuid(&value.id)?,
            name: value.name,
        })
    }
}

/// Inline AST node projected from `papo_sdk_ui::message_content::Inline`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiInline {
    /// Styled plain or safely linked text.
    Text {
        /// Literal text slice.
        text: String,
        /// Active emphasis flags.
        style: FfiTextStyle,
        /// Validated target URL (`http`, `https`, or `mailto`) when linked.
        link: Option<String>,
        /// True when Layer 2 token parsing applies inside this run.
        allow_tokens: bool,
    },
    /// Inline backtick code span.
    Code {
        /// Raw code span text.
        code: String,
    },
    /// Line break inside paragraph/heading/cell.
    LineBreak,
    /// Spoiler container (`||hidden||`).
    Spoiler {
        /// Nested inline nodes inside the spoiler.
        inlines: Vec<FfiInline>,
    },
    /// Canonical user or `@everyone` mention outside links/code.
    Mention {
        /// Canonical target identity.
        target: FfiMentionTarget,
        /// Original raw token spelling.
        raw: String,
        /// Surrounding emphasis flags.
        style: FfiTextStyle,
    },
    /// Resolved custom server emoji token outside links/code.
    CustomEmoji {
        /// Backend custom emoji UUID.
        id: String,
        /// Original `:shortcode:` token spelling.
        raw: String,
    },
}

impl From<&Inline> for FfiInline {
    fn from(value: &Inline) -> Self {
        match value {
            Inline::Text {
                text,
                style,
                link,
                allow_tokens,
            } => Self::Text {
                text: text.clone(),
                style: (*style).into(),
                link: link.clone(),
                allow_tokens: *allow_tokens,
            },
            Inline::Code(code) => Self::Code { code: code.clone() },
            Inline::LineBreak => Self::LineBreak,
            Inline::Spoiler(items) => Self::Spoiler {
                inlines: items.iter().map(Into::into).collect(),
            },
            Inline::Mention { target, raw, style } => Self::Mention {
                target: target.into(),
                raw: raw.clone(),
                style: (*style).into(),
            },
            Inline::CustomEmoji { id, raw } => Self::CustomEmoji {
                id: id.to_string(),
                raw: raw.clone(),
            },
        }
    }
}

/// Single GFM table row wrapper for `UniFFI` vector nesting.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiTableRow {
    /// Cells in column order, each holding inline AST nodes.
    pub cells: Vec<Vec<FfiInline>>,
}

/// Single list item inside `FfiBlock::List`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiListItem {
    /// `Some(checked)` for task list items; `None` for regular items.
    pub task: Option<bool>,
    /// Child blocks inside this list item.
    pub blocks: Vec<FfiBlock>,
}

impl From<&ListItem> for FfiListItem {
    fn from(value: &ListItem) -> Self {
        Self {
            task: value.task,
            blocks: value.blocks.iter().map(Into::into).collect(),
        }
    }
}

/// Top-level or nested CommonMark/GFM block produced by `MessageContent`.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FfiBlock {
    /// Paragraph of inline nodes.
    Paragraph {
        /// Inline nodes in reading order.
        inlines: Vec<FfiInline>,
    },
    /// ATX or Setext heading (`1..=6`).
    Heading {
        /// Heading level (`1..=6`).
        level: u8,
        /// Inline heading runs.
        inlines: Vec<FfiInline>,
    },
    /// Nested quote block.
    BlockQuote {
        /// Quoted child blocks.
        blocks: Vec<FfiBlock>,
    },
    /// Bullet or ordered list.
    List {
        /// Starting ordinal for ordered lists; `None` for bullet lists.
        start: Option<u64>,
        /// Items in source order.
        items: Vec<FfiListItem>,
    },
    /// Fenced or indented literal code block.
    CodeBlock {
        /// Optional trimmed language tag.
        info: Option<String>,
        /// Normalized literal code body.
        text: String,
    },
    /// Horizontal rule / thematic break.
    ThematicBreak,
    /// Structured GFM table.
    Table {
        /// Column alignments.
        alignments: Vec<FfiTableAlignment>,
        /// Header cell inlines.
        header: Vec<Vec<FfiInline>>,
        /// Body rows.
        rows: Vec<FfiTableRow>,
    },
    /// Preserved explicit blank line.
    BlankLine,
}

impl From<&Block> for FfiBlock {
    fn from(value: &Block) -> Self {
        match value {
            Block::Paragraph(inlines) => Self::Paragraph {
                inlines: inlines.iter().map(Into::into).collect(),
            },
            Block::Heading { level, inlines } => Self::Heading {
                level: *level,
                inlines: inlines.iter().map(Into::into).collect(),
            },
            Block::BlockQuote(blocks) => Self::BlockQuote {
                blocks: blocks.iter().map(Into::into).collect(),
            },
            Block::List { start, items } => Self::List {
                start: *start,
                items: items.iter().map(Into::into).collect(),
            },
            Block::CodeBlock { info, text } => Self::CodeBlock {
                info: info.clone(),
                text: text.clone(),
            },
            Block::ThematicBreak => Self::ThematicBreak,
            Block::Table {
                alignments,
                header,
                rows,
            } => Self::Table {
                alignments: alignments.iter().copied().map(Into::into).collect(),
                header: header
                    .iter()
                    .map(|cell| cell.iter().map(Into::into).collect())
                    .collect(),
                rows: rows
                    .iter()
                    .map(|row| FfiTableRow {
                        cells: row
                            .iter()
                            .map(|cell| cell.iter().map(Into::into).collect())
                            .collect(),
                    })
                    .collect(),
            },
            Block::BlankLine => Self::BlankLine,
        }
    }
}

/// Complete pre-parsed message content AST shared across timeline items and previews.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct FfiMessageContent {
    /// Parsed block tree.
    pub blocks: Vec<FfiBlock>,
    /// True if all blocks carry no visible text/tokens.
    pub is_empty: bool,
    /// Maximum nested quote/list depth in this parse.
    pub max_nesting_depth: u32,
}

impl From<&MessageContent> for FfiMessageContent {
    fn from(value: &MessageContent) -> Self {
        Self {
            blocks: value.blocks().iter().map(Into::into).collect(),
            is_empty: value.is_empty(),
            max_nesting_depth: u32::try_from(value.max_nesting_depth()).unwrap_or(u32::MAX),
        }
    }
}

/// Parses raw message text with custom server emojis (used outside cached timeline items).
///
/// # Errors
/// Returns [`FfiClientError::InvalidArgument`] if any emoji UUID is invalid.
#[uniffi::export]
pub fn parse_message_content(
    source: String,
    custom_emojis: Vec<FfiCustomEmojiRef>,
) -> Result<FfiMessageContent, FfiClientError> {
    let catalog: Vec<CustomEmojiRef> = custom_emojis
        .into_iter()
        .map(TryFrom::try_from)
        .collect::<Result<_, _>>()?;
    let parsed = MessageContent::parse_with_custom_emojis(&source, &catalog);
    Ok(FfiMessageContent::from(&parsed))
}

/// Validates whether a target URL uses a safe allowed scheme (`http`, `https`, `mailto`).
#[uniffi::export]
#[must_use]
pub fn is_safe_message_href(href: String) -> bool {
    is_safe_href(&href)
}

/// True when `text` contains no Markdown formatting or mention syntax.
#[uniffi::export]
#[must_use]
pub fn is_plain_message_text(text: String) -> bool {
    is_plain(&text)
}
