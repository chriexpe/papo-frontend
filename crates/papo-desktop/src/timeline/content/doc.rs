//! A message's content laid out as the timeline draws and copies it: the
//! SDK's parsed blocks (`MessageContent`) turned into texts with the words
//! they show and the marks over them. Pure, and the same for the view and
//! for a copy, so a copy reads exactly the texts the view drew.
//!
//! Each text has an index in document order: the view keys it `t{index}`,
//! and a copy finds it again by that key. What a mark looks like is
//! `draw`'s; what a text copies as is `copy`'s.

use std::ops::Range;

use papo_sdk::{MentionTarget, ResourceSnapshot};
use papo_sdk_ui::message_content::{Block, Inline, MessageContent, TableAlignment};

use self::build::{Builder, mention_name};
use super::emoji;

mod build;

/// Space a chip keeps on each side of its words: two narrow no-break
/// spaces, which `draw` sizes to the chip's padding and a copy leaves out.
pub(super) const PAD: &str = "\u{202f}\u{202f}";

/// A message's content, laid out.
pub(crate) struct Doc {
    pub(super) nodes: Vec<Node>,
    /// The texts' count; they are numbered from 0 in document order.
    pub(super) texts: usize,
}

/// One block.
pub(super) enum Node {
    Paragraph(Run),
    Heading {
        level: u8,
        run: Run,
    },
    Quote(Vec<Node>),
    List(Vec<Item>),
    Code {
        /// The fence's language, as written; none draws no label.
        language: Option<String>,
        run: Run,
    },
    Rule,
    Table {
        alignments: Vec<TableAlignment>,
        header: Vec<Run>,
        rows: Vec<Vec<Run>>,
    },
    /// The parser's preserved blank line.
    Blank,
    /// One to three emoji and nothing else, drawn large.
    Jumbo(Run),
}

/// One list item: its marker and its blocks.
pub(super) struct Item {
    pub(super) marker: Marker,
    pub(super) nodes: Vec<Node>,
}

/// What stands before a list item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Marker {
    /// • ◦ ▪ by nesting level, then again.
    Bullet(usize),
    /// The item's number; `width` is the widest number's digit count in
    /// its list, so the numbers share their dot.
    Number {
        value: u64,
        width: usize,
    },
    Task {
        done: bool,
    },
}

impl Marker {
    /// The marker as drawn, and as a copy writes it before the item's text.
    pub(super) fn text(self) -> String {
        match self {
            Marker::Bullet(level) => ["•", "◦", "▪"][level % 3].to_owned(),
            Marker::Number { value, .. } => format!("{value}."),
            Marker::Task { done: true } => "☑".to_owned(),
            Marker::Task { done: false } => "☐".to_owned(),
        }
    }
}

/// How a copy joins a text to the one before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Joint {
    Line,
    /// A table cell beside the one before it.
    Tab,
}

/// One text: its words as drawn, and its marks.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Run {
    pub(super) index: usize,
    pub(super) text: String,
    pub(super) marks: Vec<Mark>,
    /// What a copy writes before it when it takes the text from its start
    /// with others: a list item's marker.
    pub(super) prefix: Option<String>,
    pub(super) joint: Joint,
}

/// A styled byte range of a [`Run`]'s text.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Mark {
    Bold(Range<usize>),
    Italic(Range<usize>),
    Strike(Range<usize>),
    /// Inline code's own bytes, between its pads.
    Code(Range<usize>),
    /// A chip's padding, [`PAD`]; `pad_pt` points wide.
    Pad {
        range: Range<usize>,
        pad_pt: Pad,
    },
    /// A safe link to `url`.
    Link {
        range: Range<usize>,
        url: String,
    },
    /// A mention's label between its pads; `known` is false for a person
    /// the server does not know.
    Mention {
        range: Range<usize>,
        known: bool,
        bold: bool,
    },
    /// Spoiler `index` of its message: its words between its pads.
    Spoiler {
        range: Range<usize>,
        index: u32,
        hidden: bool,
    },
}

/// Which chip a pad belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pad {
    Chip,
    Spoiler,
}

impl Run {
    /// The byte ranges of its hidden spoilers' words.
    pub(super) fn hidden(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.marks.iter().filter_map(|mark| match mark {
            Mark::Spoiler {
                range,
                hidden: true,
                ..
            } => Some(range.clone()),
            _ => None,
        })
    }

    /// Whether byte range `range` lies in a hidden spoiler.
    pub(super) fn is_hidden(&self, range: &Range<usize>) -> bool {
        self.hidden()
            .any(|hidden| hidden.start <= range.start && range.end <= hidden.end)
    }
}

/// What laying content out reads besides the content.
pub(crate) struct Context<'a> {
    /// The account's people, for mentions' names.
    pub(crate) resources: Option<&'a ResourceSnapshot>,
    /// The spoilers revealed in this message, by index.
    pub(crate) revealed: &'a [u32],
    /// Whether one to three emoji may be drawn large (not at Extra
    /// compact).
    pub(crate) jumbo: bool,
}

impl Doc {
    /// Lays `content` out.
    pub(crate) fn new(content: &MessageContent, given: &Context<'_>) -> Doc {
        let mut builder = Builder {
            context: given,
            texts: 0,
            spoilers: 0,
        };
        let nodes = match emoji::jumbo(content).filter(|_| given.jumbo) {
            Some(emoji) => {
                let run = builder.plain(emoji.to_owned(), Joint::Line);
                vec![Node::Jumbo(run)]
            }
            None => builder.blocks(content.blocks(), 0),
        };
        Doc {
            nodes,
            texts: builder.texts,
        }
    }

    /// Whether the content ends in a block of text (a paragraph or a
    /// heading), so "(edited)" follows its last line.
    pub(crate) fn ends_in_text(&self) -> bool {
        matches!(
            self.nodes.last(),
            Some(Node::Paragraph(_) | Node::Heading { .. })
        )
    }

    /// Every text, in document order.
    pub(super) fn runs(&self) -> Vec<&Run> {
        let mut runs = Vec::with_capacity(self.texts);
        collect(&self.nodes, &mut runs);
        runs
    }
}

/// The names `content`'s mentions of people show, in order ("@Lia", none
/// for a person the account does not know): what its drawing reads from
/// the account's resources, so a row redraws when they change and not when
/// anything else in them does.
pub(crate) fn mention_names(
    content: &MessageContent,
    resources: Option<&ResourceSnapshot>,
) -> Vec<Option<String>> {
    let mut names = Vec::new();
    let mut inlines: Vec<&[Inline]> = Vec::new();
    let mut blocks: Vec<&[Block]> = vec![content.blocks()];
    while let Some(level) = blocks.pop() {
        for block in level {
            match block {
                Block::Paragraph(line) | Block::Heading { inlines: line, .. } => inlines.push(line),
                Block::BlockQuote(inner) => blocks.push(inner),
                Block::List { items, .. } => {
                    blocks.extend(items.iter().map(|item| item.blocks.as_slice()));
                }
                Block::Table { header, rows, .. } => {
                    inlines.extend(header.iter().map(Vec::as_slice));
                    for row in rows {
                        inlines.extend(row.iter().map(Vec::as_slice));
                    }
                }
                Block::CodeBlock { .. } | Block::ThematicBreak | Block::BlankLine => {}
            }
        }
    }
    while let Some(line) = inlines.pop() {
        for inline in line {
            match inline {
                Inline::Mention {
                    target: target @ MentionTarget::User(_),
                    ..
                } => names.push(mention_name(*target, resources)),
                Inline::Spoiler(inner) => inlines.push(inner),
                _ => {}
            }
        }
    }
    names
}

fn collect<'a>(nodes: &'a [Node], runs: &mut Vec<&'a Run>) {
    for node in nodes {
        match node {
            Node::Paragraph(run)
            | Node::Heading { run, .. }
            | Node::Code { run, .. }
            | Node::Jumbo(run) => runs.push(run),
            Node::Quote(nodes) => collect(nodes, runs),
            Node::List(items) => {
                for item in items {
                    collect(&item.nodes, runs);
                }
            }
            Node::Table { header, rows, .. } => {
                runs.extend(header);
                for row in rows {
                    runs.extend(row);
                }
            }
            Node::Rule | Node::Blank => {}
        }
    }
}
