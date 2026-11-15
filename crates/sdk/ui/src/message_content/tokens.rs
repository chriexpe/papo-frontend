//! Markdown-aware Layer 2 token resolution for canonical mentions and custom emoji.
//!
//! All canonical mention syntax recognition delegates to [`papo_sdk::mentions`].
//! Unlike Layer 1 notification classification (which scans raw message content context-free),
//! this pass only transforms plain/formatted text leaves outside code spans, code blocks and links.

use crate::message_content::{
    model::{Block, CustomEmojiRef, Inline, ListItem, TextStyle},
    spoiler::{InlinePiece, push_coalesced},
};
use papo_sdk::MentionTarget;

const MAX_SHORTCODE_SCAN_BYTES: usize = 128;

/// Expands Layer 2 tokens (`@mention`, `@everyone`, `:emoji:`) across parser pieces before
/// spoiler matching while keeping unsafe links and image fallbacks (`allow_tokens = false`) literal.
pub(crate) fn expand_pieces(
    pieces: Vec<InlinePiece>,
    catalog: &[CustomEmojiRef],
) -> Vec<InlinePiece> {
    let mut coalesced: Vec<InlinePiece> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        match (coalesced.last_mut(), piece) {
            (
                Some(InlinePiece::Text {
                    text: prev,
                    style: prev_style,
                    link: None,
                    allow_tokens: true,
                }),
                InlinePiece::Text {
                    text: add,
                    style,
                    link: None,
                    allow_tokens: true,
                },
            ) if *prev_style == style => prev.push_str(&add),
            (_, next) => coalesced.push(next),
        }
    }

    let mut out = Vec::with_capacity(coalesced.len());
    for piece in coalesced {
        match piece {
            InlinePiece::Text {
                text,
                style,
                link,
                allow_tokens: false,
            } => {
                if !text.is_empty() {
                    out.push(InlinePiece::Literal(Inline::Text {
                        text,
                        style,
                        link,
                        allow_tokens: false,
                    }));
                }
            }
            InlinePiece::Text {
                text,
                style,
                link: None,
                allow_tokens: true,
            } if needs_token_scan(&text, catalog) => {
                let mut tmp = Vec::new();
                split_text_tokens_with_options(&text, style, catalog, true, &mut tmp);
                for inline in tmp {
                    match inline {
                        Inline::Text {
                            text, style, link, ..
                        } => out.push(InlinePiece::Text {
                            text,
                            style,
                            link,
                            allow_tokens: true,
                        }),
                        other => out.push(InlinePiece::Literal(other)),
                    }
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Extracts canonical mentions and optional custom emoji from non-code, non-link text leaves.
pub(crate) fn apply_tokens_to_inlines(
    inlines: Vec<Inline>,
    catalog: &[CustomEmojiRef],
) -> Vec<Inline> {
    let mut out = Vec::with_capacity(inlines.len());
    for inline in inlines {
        match inline {
            Inline::Text {
                text,
                style,
                link: None,
                allow_tokens: true,
            } if needs_token_scan(&text, catalog) => {
                split_text_tokens(&text, style, catalog, &mut out);
            }
            Inline::Spoiler(inner) => {
                let resolved = apply_tokens_to_inlines(inner, catalog);
                push_coalesced(&mut out, Inline::Spoiler(resolved));
            }
            other => push_coalesced(&mut out, other),
        }
    }
    out
}

/// Applies custom emoji matching onto an existing block tree without re-parsing `CommonMark`.
pub(crate) fn apply_custom_emojis_to_blocks(
    blocks: &[Block],
    catalog: &[CustomEmojiRef],
) -> Vec<Block> {
    blocks
        .iter()
        .map(|block| match block {
            Block::Paragraph(inlines) => Block::Paragraph(apply_emoji_inlines(inlines, catalog)),
            Block::Heading { level, inlines } => Block::Heading {
                level: *level,
                inlines: apply_emoji_inlines(inlines, catalog),
            },
            Block::BlockQuote(inner) => {
                Block::BlockQuote(apply_custom_emojis_to_blocks(inner, catalog))
            }
            Block::List { start, items } => Block::List {
                start: *start,
                items: items
                    .iter()
                    .map(|item| ListItem {
                        task: item.task,
                        blocks: apply_custom_emojis_to_blocks(&item.blocks, catalog),
                    })
                    .collect(),
            },
            Block::Table {
                alignments,
                header,
                rows,
            } => Block::Table {
                alignments: alignments.clone(),
                header: header
                    .iter()
                    .map(|cell| apply_emoji_inlines(cell, catalog))
                    .collect(),
                rows: rows
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|cell| apply_emoji_inlines(cell, catalog))
                            .collect()
                    })
                    .collect(),
            },
            other => other.clone(),
        })
        .collect()
}

fn apply_emoji_inlines(inlines: &[Inline], catalog: &[CustomEmojiRef]) -> Vec<Inline> {
    let mut out = Vec::with_capacity(inlines.len());
    for inline in inlines {
        match inline {
            Inline::Text {
                text,
                style,
                link: None,
                allow_tokens: true,
            } if text.contains(':') => {
                split_text_tokens_with_options(text, *style, catalog, false, &mut out);
            }
            Inline::Spoiler(inner) => {
                push_coalesced(
                    &mut out,
                    Inline::Spoiler(apply_emoji_inlines(inner, catalog)),
                );
            }
            other => push_coalesced(&mut out, other.clone()),
        }
    }
    out
}

fn needs_token_scan(text: &str, catalog: &[CustomEmojiRef]) -> bool {
    text.contains('@') || (!catalog.is_empty() && text.contains(':'))
}

fn split_text_tokens(
    text: &str,
    style: TextStyle,
    catalog: &[CustomEmojiRef],
    out: &mut Vec<Inline>,
) {
    split_text_tokens_with_options(text, style, catalog, true, out);
}

fn split_text_tokens_with_options(
    text: &str,
    style: TextStyle,
    catalog: &[CustomEmojiRef],
    scan_mentions: bool,
    out: &mut Vec<Inline>,
) {
    let bytes = text.as_bytes();
    let mut plain_start = 0usize;
    let mut index = 0usize;

    while index < bytes.len() {
        if scan_mentions && bytes[index] == b'@' {
            if let Some((id, end)) = papo_sdk::mentions::parse_user_mention_at(text, index) {
                flush_plain(text, plain_start..index, style, out);
                out.push(Inline::Mention {
                    target: MentionTarget::User(id),
                    raw: text[index..end].to_owned(),
                    style,
                });
                index = end;
                plain_start = end;
                continue;
            }
            if let Some(end) = papo_sdk::mentions::parse_everyone_at(text, index, true) {
                flush_plain(text, plain_start..index, style, out);
                out.push(Inline::Mention {
                    target: MentionTarget::Everyone,
                    raw: text[index..end].to_owned(),
                    style,
                });
                index = end;
                plain_start = end;
                continue;
            }
        } else if bytes[index] == b':'
            && !catalog.is_empty()
            && let Some((emoji, end)) = match_custom_emoji_at(text, index, catalog)
        {
            flush_plain(text, plain_start..index, style, out);
            out.push(Inline::CustomEmoji {
                id: emoji.id,
                raw: text[index..end].to_owned(),
            });
            index = end;
            plain_start = end;
            continue;
        }
        index += 1;
    }

    flush_plain(text, plain_start..bytes.len(), style, out);
}

fn flush_plain(text: &str, range: std::ops::Range<usize>, style: TextStyle, out: &mut Vec<Inline>) {
    if range.start < range.end {
        push_coalesced(
            out,
            Inline::Text {
                text: text[range].to_owned(),
                style,
                link: None,
                allow_tokens: true,
            },
        );
    }
}

fn match_custom_emoji_at<'a>(
    text: &str,
    start: usize,
    catalog: &'a [CustomEmojiRef],
) -> Option<(&'a CustomEmojiRef, usize)> {
    let after = text.get(start + 1..)?;
    let search_limit = after.len().min(MAX_SHORTCODE_SCAN_BYTES + 1);
    let mut close_rel = None;
    for (offset, ch) in after.char_indices() {
        if offset >= search_limit {
            break;
        }
        if ch == ':' {
            close_rel = Some(offset);
            break;
        }
        if ch.is_whitespace() || ch.is_control() {
            return None;
        }
    }
    let close = close_rel?;
    if close == 0 {
        return None;
    }
    let candidate = &after[..close];
    let entry = catalog.iter().find(|entry| entry.name == candidate)?;
    Some((entry, start + 1 + close + 1))
}
