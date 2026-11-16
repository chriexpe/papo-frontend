//! Framework-neutral parsed message content tree.
//!
//! Owns the structural block/inline representation for rendered messages without
//! any UI-framework types, display names, resolved colours or media assets.

use papo_sdk::{EmojiMetadata, MentionTarget, Uuid};

use crate::message_content::{parser, tokens};

/// Maximum nesting depth permitted across blocks and inline spoilers.
pub const MAX_NESTING_DEPTH: usize = 64;

/// Cumulative inline text formatting active on a text or mention leaf.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TextStyle {
    /// Double-asterisk or double-underscore strong emphasis (`**text**`, `__text__`).
    pub bold: bool,
    /// Single-asterisk or single-underscore emphasis (`*text*`, `_text_`).
    pub italic: bool,
    /// Single- or double-tilde strikethrough (`~text~`, `~~text~~`).
    pub strikethrough: bool,
}

/// Column alignment of a GFM table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TableAlignment {
    /// Left-aligned or unspecified column (`:--` or `---`).
    #[default]
    Left,
    /// Center-aligned column (`:-:`).
    Center,
    /// Right-aligned column (`--:`).
    Right,
}

/// Minimal caller-supplied custom emoji entry used to tag `:shortcode:` tokens by ID.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CustomEmojiRef {
    /// Backend UUID of the custom emoji.
    pub id: Uuid,
    /// Shortcode name without surrounding colons.
    pub name: String,
}

impl From<&EmojiMetadata> for CustomEmojiRef {
    fn from(meta: &EmojiMetadata) -> Self {
        Self {
            id: meta.id,
            name: meta.name.clone(),
        }
    }
}

/// One inline leaf or container inside a paragraph, heading or table cell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Inline {
    /// Styled plain or linked text (never contains UI colours or media).
    Text {
        /// Literal text slice.
        text: String,
        /// Active cumulative bold/italic/strikethrough flags.
        style: TextStyle,
        /// Safe target URL (`http`, `https` or `mailto` only).
        link: Option<String>,
        /// Whether Layer 2 token scans (mentions, custom emoji) may expand inside this text.
        ///
        /// False for safe links, rejected unsafe links and image fallbacks so
        /// [`MessageContent::with_custom_emojis`] leaves those ranges literal just like
        /// [`MessageContent::parse_with_custom_emojis`].
        allow_tokens: bool,
    },
    /// Inline code span (backtick-delimited); mentions, spoilers and emoji remain literal inside.
    Code(String),
    /// Explicit or single-newline line break.
    LineBreak,
    /// Inline spoiler (`||hidden||`).
    Spoiler(Vec<Inline>),
    /// Canonical mention recognized outside code spans and links.
    Mention {
        /// Canonical target (`User(Uuid)` or `Everyone`).
        target: MentionTarget,
        /// Exact raw token text preserved verbatim from the source message.
        raw: String,
        /// Cumulative bold/italic/strikethrough flags surrounding the token.
        style: TextStyle,
    },
    /// Custom server emoji resolved to its canonical ID outside code spans and links.
    CustomEmoji {
        /// Backend UUID of the matched custom emoji.
        id: Uuid,
        /// Exact raw `:shortcode:` token preserved from the source message.
        raw: String,
    },
}

/// Single bullet, ordered or task item inside [`Block::List`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListItem {
    /// `Some(checked)` for GFM task items (`- [x]` / `- [ ]`), `None` for regular items.
    pub task: Option<bool>,
    /// Child blocks inside this list item.
    pub blocks: Vec<Block>,
}

/// Top-level or container Markdown block.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Block {
    /// Paragraph of inline runs.
    Paragraph(Vec<Inline>),
    /// ATX or Setext heading (`level` in `1..=6`).
    Heading {
        /// Heading level (`1..=6`).
        level: u8,
        /// Heading inline runs.
        inlines: Vec<Inline>,
    },
    /// Quoted nested blocks (`> ...`).
    BlockQuote(Vec<Block>),
    /// Bullet or ordered list (`start = Some(n)` when ordered).
    List {
        /// Starting ordinal for ordered lists, or `None` for bullet lists.
        start: Option<u64>,
        /// List items in source order.
        items: Vec<ListItem>,
    },
    /// Fenced or indented code block preserving any fenced info string.
    CodeBlock {
        /// Optional trimmed fence info string (for example `"rust"` or `"json"`).
        info: Option<String>,
        /// Literal code contents (newlines normalized, trailing block newline trimmed).
        text: String,
    },
    /// Thematic break (`---`, `***`).
    ThematicBreak,
    /// GFM table.
    Table {
        /// Column alignments from the delimiter row.
        alignments: Vec<TableAlignment>,
        /// Header row cells.
        header: Vec<Vec<Inline>>,
        /// Body rows of inline cells.
        rows: Vec<Vec<Vec<Inline>>>,
    },
    /// Preserved extra blank line outside code blocks.
    BlankLine,
}

/// Parsed framework-neutral representation of one message revision.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MessageContent {
    /// Top-level blocks in source order.
    pub blocks: Vec<Block>,
}

impl MessageContent {
    /// Parses Markdown message content into a bounded, framework-neutral block/inline tree.
    #[must_use]
    pub fn parse(source: &str) -> Self {
        parser::parse(source, &[])
    }

    /// Parses Markdown message content while resolving `:shortcode:` custom emoji against `catalog`.
    #[must_use]
    pub fn parse_with_custom_emojis(source: &str, catalog: &[CustomEmojiRef]) -> Self {
        parser::parse(source, catalog)
    }

    /// Re-scans token-eligible plain text leaves in an already-parsed tree against `catalog`
    /// without re-parsing Markdown.
    ///
    /// Text belonging to links (safe or rejected) and image fallbacks carries `allow_tokens: false`
    /// and stays literal, matching [`MessageContent::parse_with_custom_emojis`].
    #[must_use]
    pub fn with_custom_emojis(&self, catalog: &[CustomEmojiRef]) -> Self {
        if catalog.is_empty() || self.blocks.is_empty() {
            return self.clone();
        }
        Self {
            blocks: tokens::apply_custom_emojis_to_blocks(&self.blocks, catalog),
        }
    }

    /// Returns the parsed blocks in source order.
    #[must_use]
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Returns `true` when parsing produced no blocks (empty input or dropped raw HTML only).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    /// Computes the maximum structural nesting depth across all blocks and inline trees.
    #[must_use]
    pub fn max_nesting_depth(&self) -> usize {
        enum Frame<'a> {
            Blocks(&'a [Block], usize),
            Inlines(&'a [Inline], usize),
        }

        let mut stack = vec![Frame::Blocks(&self.blocks, 0)];
        let mut max_depth = 0usize;

        while let Some(frame) = stack.pop() {
            match frame {
                Frame::Blocks(blocks, depth) => {
                    for block in blocks {
                        let next_depth = depth + 1;
                        max_depth = max_depth.max(next_depth);
                        match block {
                            Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                                stack.push(Frame::Inlines(inlines, next_depth));
                            }
                            Block::BlockQuote(inner) => {
                                stack.push(Frame::Blocks(inner, next_depth));
                            }
                            Block::List { items, .. } => {
                                let item_depth = next_depth + 1;
                                if !items.is_empty() {
                                    max_depth = max_depth.max(item_depth);
                                }
                                for item in items {
                                    stack.push(Frame::Blocks(&item.blocks, item_depth));
                                }
                            }
                            Block::Table { header, rows, .. } => {
                                for cell in header {
                                    stack.push(Frame::Inlines(cell, next_depth));
                                }
                                for row in rows {
                                    for cell in row {
                                        stack.push(Frame::Inlines(cell, next_depth));
                                    }
                                }
                            }
                            Block::CodeBlock { .. } | Block::ThematicBreak | Block::BlankLine => {}
                        }
                    }
                }
                Frame::Inlines(inlines, depth) => {
                    for inline in inlines {
                        let next_depth = depth + 1;
                        max_depth = max_depth.max(next_depth);
                        if let Inline::Spoiler(inner) = inline {
                            stack.push(Frame::Inlines(inner, next_depth));
                        }
                    }
                }
            }
        }

        max_depth
    }
}
