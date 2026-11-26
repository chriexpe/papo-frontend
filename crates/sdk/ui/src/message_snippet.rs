//! A message summed up in one line, for a reply line, a reply bar and any other one-line
//! preview. Owns only the choice of what to show; the words ("Image", "File · name"), names of
//! mentioned people, styling and truncation are the client's.

use papo_sdk::{MentionTarget, Message, Uuid};

use crate::attachment_kind::AttachmentKind;
use crate::message_content::{Block, Inline, MessageContent};

/// What one line can say about a message, in order of preference: its first line of text,
/// else its first attachment, else its first embed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Snippet {
    /// The first non-empty line of text, never empty.
    Text(Vec<SnippetPart>),
    /// The first attachment is a GIF, or the message is one GIF link it hides
    /// (`link_preview::hides_text`).
    Gif,
    /// The first attachment is a JPEG, PNG or WebP picture (`AttachmentKind::Image`).
    Image,
    /// The first attachment is a video.
    Video,
    /// The first attachment is audio (`AttachmentKind::Audio`).
    Audio,
    /// The first attachment is another file.
    File {
        /// Its original file name, as uploaded.
        name: String,
    },
    /// A message that is only a link (or only an embed): its embed's title and the link's
    /// host.
    Link {
        /// The embed's title, when it has one.
        title: Option<String>,
        /// The host the link points at, when it parses.
        domain: Option<String>,
    },
    /// Nothing to show.
    Empty,
}

/// One piece of a text snippet.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnippetPart {
    /// Plain text, Markdown already removed.
    Text(String),
    /// A mention; the client names it.
    Mention(MentionTarget),
    /// A custom emoji, by its ID and its `:shortcode:`.
    CustomEmoji {
        /// The emoji's backend UUID.
        id: Uuid,
        /// Its `:shortcode:` as written.
        raw: String,
    },
    /// A spoiler, kept hidden: its text is not given.
    Spoiler,
}

/// Sums `message` (whose parsed content is `content`) up in one line.
#[must_use]
pub fn snippet(message: &Message, content: &MessageContent) -> Snippet {
    // A GIF-only message shows only its GIF, so its line says so, as a GIF attachment's does
    // (`docs/design.md` §16, "A GIF-only message hides its link").
    if crate::link_preview::hides_text(message) {
        return Snippet::Gif;
    }
    if let Some(link) = only_link(content) {
        return link_snippet(message, Some(&link));
    }
    let mut line = Line::default();
    blocks(&content.blocks, &mut line);
    if !line.parts.is_empty() {
        return Snippet::Text(line.finish());
    }
    if let Some(attachment) = message.attachments.as_deref().and_then(<[_]>::first) {
        // The word follows what the message draws.
        return match AttachmentKind::of(attachment) {
            AttachmentKind::Gif => Snippet::Gif,
            AttachmentKind::Image => Snippet::Image,
            AttachmentKind::Video => Snippet::Video,
            AttachmentKind::Audio => Snippet::Audio,
            AttachmentKind::File => Snippet::File {
                name: attachment.original_file_name.clone().unwrap_or_default(),
            },
        };
    }
    if message
        .embeds
        .as_deref()
        .is_some_and(|embeds| !embeds.is_empty())
    {
        return link_snippet(message, None);
    }
    Snippet::Empty
}

/// The first embed's title and the host of `link`, else of the embed's own address.
fn link_snippet(message: &Message, link: Option<&str>) -> Snippet {
    let embed = message.embeds.as_deref().and_then(<[_]>::first);
    let address = link.or_else(|| embed.and_then(|embed| embed.url.as_deref()));
    Snippet::Link {
        title: embed
            .and_then(|embed| embed.title.clone())
            .filter(|title| !title.trim().is_empty()),
        domain: address
            .and_then(|address| url::Url::parse(address).ok())
            .and_then(|url| url.host_str().map(str::to_owned)),
    }
}

/// The address of a message that is a single link and nothing else: a Markdown link, or a
/// bare `http`/`https` address (the parser leaves those as text).
fn only_link(content: &MessageContent) -> Option<String> {
    let [Block::Paragraph(inlines)] = content.blocks.as_slice() else {
        return None;
    };
    let mut address: Option<&str> = None;
    for inline in inlines {
        let found = match inline {
            Inline::Text {
                link: Some(link), ..
            } => link.as_str(),
            Inline::Text {
                text, link: None, ..
            } if text.trim().is_empty() => continue,
            Inline::Text {
                text, link: None, ..
            } if is_bare_address(text.trim()) => text.trim(),
            _ => return None,
        };
        if address.is_some_and(|known| known != found) {
            return None;
        }
        address = Some(found);
    }
    address.map(str::to_owned)
}

fn is_bare_address(text: &str) -> bool {
    !text.contains(char::is_whitespace)
        && url::Url::parse(text)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host().is_some())
}

/// A line being collected; it ends at the first line break after some content.
#[derive(Default)]
struct Line {
    parts: Vec<SnippetPart>,
    ended: bool,
}

impl Line {
    fn text(&mut self, text: &str) {
        let mut segments = text.split('\n');
        if let Some(first) = segments.next() {
            self.push_text(first);
        }
        for segment in segments {
            // A newline ends a line that holds something; blank lines before it are skipped.
            if self.has_content() {
                self.ended = true;
                return;
            }
            self.parts.clear();
            self.push_text(segment);
        }
    }

    fn push_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Some(SnippetPart::Text(last)) = self.parts.last_mut() {
            last.push_str(text);
        } else {
            self.parts.push(SnippetPart::Text(text.to_owned()));
        }
    }

    fn push(&mut self, part: SnippetPart) {
        self.parts.push(part);
    }

    fn has_content(&self) -> bool {
        self.parts.iter().any(|part| match part {
            SnippetPart::Text(text) => !text.trim().is_empty(),
            _ => true,
        })
    }

    /// Ends a block: the line ends there if it holds anything.
    fn block_end(&mut self) {
        if self.has_content() {
            self.ended = true;
        } else {
            self.parts.clear();
        }
    }

    /// The parts with the line's outer whitespace trimmed and empty text dropped.
    fn finish(mut self) -> Vec<SnippetPart> {
        if let Some(SnippetPart::Text(first)) = self.parts.first_mut() {
            *first = first.trim_start().to_owned();
        }
        if let Some(SnippetPart::Text(last)) = self.parts.last_mut() {
            *last = last.trim_end().to_owned();
        }
        self.parts
            .retain(|part| !matches!(part, SnippetPart::Text(text) if text.is_empty()));
        self.parts
    }
}

fn blocks(blocks: &[Block], line: &mut Line) {
    for block in blocks {
        if line.ended {
            return;
        }
        match block {
            Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                self::inlines(inlines, line);
                line.block_end();
            }
            Block::BlockQuote(children) => self::blocks(children, line),
            Block::List { items, .. } => {
                for item in items {
                    self::blocks(&item.blocks, line);
                    if line.ended {
                        return;
                    }
                }
            }
            Block::CodeBlock { text, .. } => {
                line.text(text);
                line.block_end();
            }
            Block::Table { header, rows, .. } => {
                for cell in header.iter().chain(rows.iter().flatten()) {
                    if line.has_content() {
                        line.push_text(" ");
                    }
                    self::inlines(cell, line);
                }
                line.block_end();
            }
            Block::ThematicBreak | Block::BlankLine => {}
        }
    }
}

fn inlines(inlines: &[Inline], line: &mut Line) {
    for inline in inlines {
        if line.ended {
            return;
        }
        match inline {
            Inline::Text { text, .. } | Inline::Code(text) => line.text(text),
            Inline::LineBreak => {
                if line.has_content() {
                    line.ended = true;
                }
            }
            Inline::Spoiler(_) => line.push(SnippetPart::Spoiler),
            Inline::Mention { target, .. } => line.push(SnippetPart::Mention(*target)),
            Inline::CustomEmoji { id, raw } => line.push(SnippetPart::CustomEmoji {
                id: *id,
                raw: raw.clone(),
            }),
        }
    }
}

#[cfg(test)]
mod tests;
