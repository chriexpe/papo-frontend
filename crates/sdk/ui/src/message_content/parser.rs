//! Bounded `CommonMark` + GFM + spoiler parser producing [`MessageContent`].

use std::ops::Range;

use pulldown_cmark::{Alignment, Event, Parser, Tag, TagEnd};

use crate::message_content::{
    fences::{normalize_fences, read_code_block},
    model::{
        Block, CustomEmojiRef, Inline, ListItem, MAX_NESTING_DEPTH, MessageContent, TableAlignment,
        TextStyle,
    },
    preprocess::{
        CheckedOffsetEvents, escape_plus_list_markers, heading_level, is_inline_end, is_safe_href,
        markdown_options, normalize_newlines, odd_trailing_backslashes, preserve_blank_lines,
    },
    spoiler::{InlinePiece, resolve_spoilers},
    tokens::{apply_tokens_to_inlines, expand_pieces},
};

/// Parses Markdown `source` into [`MessageContent`] and resolves Layer 2 mentions/custom emoji.
pub(crate) fn parse(source: &str, custom_emojis: &[CustomEmojiRef]) -> MessageContent {
    if source.is_empty() {
        return MessageContent::default();
    }

    let normalized_newlines = normalize_newlines(source);
    let normalized_fences = normalize_fences(&normalized_newlines);
    let (with_blanks, blank_marker) = preserve_blank_lines(&normalized_fences);
    let options = markdown_options();
    let prepared = escape_plus_list_markers(&with_blanks, options);

    let mut events = CheckedOffsetEvents {
        inner: Parser::new_ext(&prepared, options).into_offset_iter(),
        depth: 0,
        exceeded: false,
    };
    let mut builder = Builder {
        source: &prepared,
        blank_marker: &blank_marker,
        custom_emojis,
        bold: 0,
        italic: 0,
        strikethrough: 0,
        links: Vec::new(),
        image_alt: Vec::new(),
        task: None,
        block_depth: 0,
    };

    let blocks = builder.blocks(&mut events);
    if events.exceeded {
        return literal_fallback(source, custom_emojis);
    }

    let content = MessageContent { blocks };
    if content.max_nesting_depth() > MAX_NESTING_DEPTH {
        return literal_fallback(source, custom_emojis);
    }
    content
}

fn literal_fallback(source: &str, custom_emojis: &[CustomEmojiRef]) -> MessageContent {
    let text = normalize_newlines(source).into_owned();
    let inlines = apply_tokens_to_inlines(
        vec![Inline::Text {
            text,
            style: TextStyle::default(),
            link: None,
            allow_tokens: true,
        }],
        custom_emojis,
    );
    if inlines.is_empty() {
        return MessageContent::default();
    }
    MessageContent {
        blocks: vec![Block::Paragraph(inlines)],
    }
}

struct Builder<'a> {
    source: &'a str,
    blank_marker: &'a str,
    custom_emojis: &'a [CustomEmojiRef],
    bold: u32,
    italic: u32,
    strikethrough: u32,
    links: Vec<Option<String>>,
    image_alt: Vec<(String, String)>,
    task: Option<bool>,
    block_depth: usize,
}

impl Builder<'_> {
    const fn style(&self) -> TextStyle {
        TextStyle {
            bold: self.bold > 0,
            italic: self.italic > 0,
            strikethrough: self.strikethrough > 0,
        }
    }

    fn link(&self) -> Option<String> {
        self.links.iter().rev().find_map(Clone::clone)
    }

    const fn allow_tokens(&self) -> bool {
        self.links.is_empty()
    }

    fn finish_inlines(&self, pieces: Vec<InlinePiece>) -> Vec<Inline> {
        let expanded = expand_pieces(pieces, self.custom_emojis);
        // Reserve room for enclosing inline block + leaf Text/Mention node inside Spoiler.
        let spoiler_budget = MAX_NESTING_DEPTH.saturating_sub(self.block_depth + 2);
        resolve_spoilers(expanded, spoiler_budget)
    }

    fn flush_pending(&self, pending: &mut Vec<InlinePiece>, blocks: &mut Vec<Block>) {
        if pending.is_empty() {
            return;
        }
        let inlines = self.finish_inlines(std::mem::take(pending));
        if !inlines.is_empty() {
            blocks.push(Block::Paragraph(inlines));
        }
    }

    fn paragraph_or_blank(&self, pieces: Vec<InlinePiece>) -> Block {
        let inlines = self.finish_inlines(pieces);
        if !self.blank_marker.is_empty()
            && let [Inline::Text { text, .. }] = inlines.as_slice()
            && text == self.blank_marker
        {
            return Block::BlankLine;
        }
        Block::Paragraph(inlines)
    }

    fn blocks<'e>(
        &mut self,
        events: &mut impl Iterator<Item = (Event<'e>, Range<usize>)>,
    ) -> Vec<Block> {
        let mut blocks = Vec::new();
        let mut pending: Vec<InlinePiece> = Vec::new();

        while let Some((event, range)) = events.next() {
            match event {
                Event::End(end) if !is_inline_end(end) => break,
                Event::Start(Tag::Paragraph) => {
                    self.flush_pending(&mut pending, &mut blocks);
                    self.block_depth += 1;
                    let pieces = self.inline_pieces(events);
                    let block = self.paragraph_or_blank(pieces);
                    self.block_depth = self.block_depth.saturating_sub(1);
                    blocks.push(block);
                }
                Event::Start(Tag::Heading { level, .. }) => {
                    self.flush_pending(&mut pending, &mut blocks);
                    self.block_depth += 1;
                    let pieces = self.inline_pieces(events);
                    let inlines = self.finish_inlines(pieces);
                    self.block_depth = self.block_depth.saturating_sub(1);
                    blocks.push(Block::Heading {
                        level: heading_level(level),
                        inlines,
                    });
                }
                Event::Start(Tag::BlockQuote(_)) => {
                    self.flush_pending(&mut pending, &mut blocks);
                    self.block_depth += 1;
                    let inner = self.blocks(events);
                    self.block_depth = self.block_depth.saturating_sub(1);
                    blocks.push(Block::BlockQuote(inner));
                }
                Event::Start(Tag::CodeBlock(kind)) => {
                    self.flush_pending(&mut pending, &mut blocks);
                    blocks.push(read_code_block(kind, events));
                }
                Event::Start(Tag::List(start)) => {
                    self.flush_pending(&mut pending, &mut blocks);
                    blocks.push(self.list(start, events));
                }
                Event::Start(Tag::Table(aligns)) => {
                    self.flush_pending(&mut pending, &mut blocks);
                    self.block_depth += 1;
                    let alignments = aligns
                        .iter()
                        .map(|align| match align {
                            Alignment::Center => TableAlignment::Center,
                            Alignment::Right => TableAlignment::Right,
                            Alignment::Left | Alignment::None => TableAlignment::Left,
                        })
                        .collect();
                    let table = self.table(alignments, events);
                    self.block_depth = self.block_depth.saturating_sub(1);
                    blocks.push(table);
                }
                Event::Start(Tag::HtmlBlock) => {
                    self.flush_pending(&mut pending, &mut blocks);
                    for (html_event, _) in events.by_ref() {
                        if matches!(html_event, Event::End(TagEnd::HtmlBlock)) {
                            break;
                        }
                    }
                }
                Event::Rule => {
                    self.flush_pending(&mut pending, &mut blocks);
                    blocks.push(Block::ThematicBreak);
                }
                other => {
                    self.push_inline_event(other, range, &mut pending);
                }
            }
        }

        self.flush_pending(&mut pending, &mut blocks);
        blocks
    }

    fn list<'e>(
        &mut self,
        start: Option<u64>,
        events: &mut impl Iterator<Item = (Event<'e>, Range<usize>)>,
    ) -> Block {
        self.block_depth += 1;
        let mut items = Vec::new();
        while let Some((list_event, _)) = events.next() {
            match list_event {
                Event::Start(Tag::Item) => {
                    let outer_task = self.task.take();
                    self.block_depth += 1;
                    let inner = self.blocks(events);
                    self.block_depth = self.block_depth.saturating_sub(1);
                    let task = self.task.take();
                    self.task = outer_task;
                    items.push(ListItem {
                        task,
                        blocks: inner,
                    });
                }
                Event::End(_) => break,
                _ => {}
            }
        }
        self.block_depth = self.block_depth.saturating_sub(1);
        Block::List { start, items }
    }

    fn table<'e>(
        &mut self,
        alignments: Vec<TableAlignment>,
        events: &mut impl Iterator<Item = (Event<'e>, Range<usize>)>,
    ) -> Block {
        let mut header = Vec::new();
        let mut rows: Vec<Vec<Vec<Inline>>> = Vec::new();
        let mut row: Vec<Vec<Inline>> = Vec::new();

        while let Some((event, _)) = events.next() {
            match event {
                Event::Start(Tag::TableRow) => row = Vec::new(),
                Event::Start(Tag::TableCell) => {
                    let pieces = self.inline_pieces(events);
                    row.push(self.finish_inlines(pieces));
                }
                Event::End(TagEnd::TableHead) => header = std::mem::take(&mut row),
                Event::End(TagEnd::TableRow) => rows.push(std::mem::take(&mut row)),
                Event::End(TagEnd::Table) => break,
                _ => {}
            }
        }

        Block::Table {
            alignments,
            header,
            rows,
        }
    }

    fn inline_pieces<'e>(
        &mut self,
        events: &mut impl Iterator<Item = (Event<'e>, Range<usize>)>,
    ) -> Vec<InlinePiece> {
        let mut out = Vec::new();
        for (event, range) in events.by_ref() {
            if matches!(&event, Event::End(end) if !is_inline_end(*end)) {
                break;
            }
            self.push_inline_event(event, range, &mut out);
        }
        out
    }

    fn push_inline_event(
        &mut self,
        event: Event<'_>,
        range: Range<usize>,
        out: &mut Vec<InlinePiece>,
    ) {
        match event {
            Event::Text(text) => {
                if let Some((_, alt)) = self.image_alt.last_mut() {
                    alt.push_str(&text);
                    return;
                }
                let leading_escaped_pipe = text.starts_with('|')
                    && (odd_trailing_backslashes(&self.source.as_bytes()[..range.start])
                        || (self.source.get(range.clone()) != Some(text.as_ref())
                            && !self.source.get(range).is_some_and(|s| s.starts_with('|'))));
                if leading_escaped_pipe {
                    out.push(InlinePiece::EscapedPipe {
                        style: self.style(),
                        link: self.link(),
                        allow_tokens: self.allow_tokens(),
                    });
                    if text.len() > 1 {
                        out.push(InlinePiece::Text {
                            text: text[1..].to_owned(),
                            style: self.style(),
                            link: self.link(),
                            allow_tokens: self.allow_tokens(),
                        });
                    }
                } else {
                    out.push(InlinePiece::Text {
                        text: text.into_string(),
                        style: self.style(),
                        link: self.link(),
                        allow_tokens: self.allow_tokens(),
                    });
                }
            }
            Event::Code(code) => {
                if let Some((_, alt)) = self.image_alt.last_mut() {
                    alt.push_str(&code);
                } else {
                    out.push(InlinePiece::Code(code.into_string()));
                }
            }
            Event::SoftBreak | Event::HardBreak => {
                if let Some((_, alt)) = self.image_alt.last_mut() {
                    alt.push(' ');
                } else {
                    out.push(InlinePiece::LineBreak);
                }
            }
            Event::TaskListMarker(checked) => self.task = Some(checked),
            Event::Start(Tag::Strong) => self.bold += 1,
            Event::End(TagEnd::Strong) => self.bold = self.bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => self.italic += 1,
            Event::End(TagEnd::Emphasis) => self.italic = self.italic.saturating_sub(1),
            Event::Start(Tag::Strikethrough) => self.strikethrough += 1,
            Event::End(TagEnd::Strikethrough) => {
                self.strikethrough = self.strikethrough.saturating_sub(1);
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                let href = dest_url.into_string();
                self.links.push(is_safe_href(&href).then_some(href));
            }
            Event::End(TagEnd::Link) => {
                self.links.pop();
            }
            Event::Start(Tag::Image { dest_url, .. }) => {
                self.image_alt.push((dest_url.into_string(), String::new()));
            }
            Event::End(TagEnd::Image) => {
                if let Some((src, alt)) = self.image_alt.pop() {
                    // No media nodes exist in the tree: fall back to safe link or alt/URL text.
                    let text = if alt.is_empty() { src.clone() } else { alt };
                    let link = is_safe_href(&src).then_some(src);
                    out.push(InlinePiece::Text {
                        text,
                        style: self.style(),
                        link,
                        allow_tokens: false,
                    });
                }
            }
            _ => {}
        }
    }
}
