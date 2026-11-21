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
use papo_sdk_ui::message_content::{
    Block, Inline, ListItem, MessageContent, TableAlignment, TextStyle,
};

use super::emoji;
use crate::strings;

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

/// Numbers texts and spoilers as it lays blocks out.
struct Builder<'a, 'c> {
    context: &'a Context<'c>,
    texts: usize,
    spoilers: u32,
}

impl Builder<'_, '_> {
    fn blocks(&mut self, blocks: &[Block], lists: usize) -> Vec<Node> {
        blocks
            .iter()
            .map(|block| self.block(block, lists))
            .collect()
    }

    fn block(&mut self, block: &Block, lists: usize) -> Node {
        match block {
            Block::Paragraph(inlines) => Node::Paragraph(self.run(inlines, Joint::Line)),
            Block::Heading { level, inlines } => Node::Heading {
                level: (*level).clamp(1, 6),
                run: self.run(inlines, Joint::Line),
            },
            Block::BlockQuote(blocks) => Node::Quote(self.blocks(blocks, lists)),
            Block::List { start, items } => Node::List(self.list(*start, items, lists)),
            Block::CodeBlock { info, text } => Node::Code {
                language: info
                    .as_deref()
                    .and_then(|info| info.split_whitespace().next())
                    .map(str::to_owned),
                run: self.plain(text.clone(), Joint::Line),
            },
            Block::ThematicBreak => Node::Rule,
            Block::Table {
                alignments,
                header,
                rows,
            } => Node::Table {
                alignments: alignments.clone(),
                header: self.cells(header),
                rows: rows.iter().map(|row| self.cells(row)).collect(),
            },
            Block::BlankLine => Node::Blank,
        }
    }

    fn list(&mut self, start: Option<u64>, items: &[ListItem], lists: usize) -> Vec<Item> {
        let width = start.map_or(0, |start| {
            let last = start.saturating_add(items.len().saturating_sub(1) as u64);
            last.to_string().len()
        });
        items
            .iter()
            .enumerate()
            .map(|(n, item)| {
                let marker = match (item.task, start) {
                    (Some(done), _) => Marker::Task { done },
                    (None, Some(start)) => Marker::Number {
                        value: start.saturating_add(n as u64),
                        width,
                    },
                    (None, None) => Marker::Bullet(lists),
                };
                let first = self.texts;
                let mut nodes = self.blocks(&item.blocks, lists + 1);
                if let Some(run) = first_run(&mut nodes).filter(|run| run.index == first) {
                    run.prefix = Some(format!("{} ", marker.text()));
                }
                Item { marker, nodes }
            })
            .collect()
    }

    fn cells(&mut self, cells: &[Vec<Inline>]) -> Vec<Run> {
        cells
            .iter()
            .enumerate()
            .map(|(column, inlines)| {
                let joint = if column == 0 { Joint::Line } else { Joint::Tab };
                self.run(inlines, joint)
            })
            .collect()
    }

    /// A text without marks.
    fn plain(&mut self, text: String, joint: Joint) -> Run {
        let index = self.next_text();
        Run {
            index,
            text,
            marks: Vec::new(),
            prefix: None,
            joint,
        }
    }

    fn run(&mut self, inlines: &[Inline], joint: Joint) -> Run {
        let index = self.next_text();
        let mut run = Run {
            index,
            text: String::new(),
            marks: Vec::new(),
            prefix: None,
            joint,
        };
        self.inlines(&mut run, inlines);
        run
    }

    fn next_text(&mut self) -> usize {
        self.texts += 1;
        self.texts - 1
    }

    /// Appends `inlines` to `run`.
    fn inlines(&mut self, run: &mut Run, inlines: &[Inline]) {
        for inline in inlines {
            match inline {
                Inline::Text {
                    text, style, link, ..
                } => {
                    let range = push(run, text);
                    styles(run, &range, *style);
                    if let Some(url) = link {
                        run.marks.push(Mark::Link {
                            range,
                            url: url.clone(),
                        });
                    }
                }
                Inline::Code(code) => {
                    pad(run, Pad::Chip);
                    let range = push(run, code);
                    run.marks.push(Mark::Code(range));
                    pad(run, Pad::Chip);
                }
                Inline::LineBreak => {
                    push(run, "\n");
                }
                Inline::Spoiler(inner) => {
                    let index = self.spoilers;
                    self.spoilers += 1;
                    let shut = !self.context.revealed.contains(&index);
                    pad(run, Pad::Spoiler);
                    let start = run.text.len();
                    self.inlines(run, inner);
                    let range = start..run.text.len();
                    pad(run, Pad::Spoiler);
                    run.marks.push(Mark::Spoiler {
                        range,
                        index,
                        hidden: shut,
                    });
                }
                Inline::Mention { target, style, .. } => {
                    let name = mention_name(*target, self.context.resources);
                    pad(run, Pad::Chip);
                    let label = name.as_deref().unwrap_or_else(|| strings::unknown_person());
                    let range = push(run, label);
                    run.marks.push(Mark::Mention {
                        range: range.clone(),
                        known: name.is_some(),
                        bold: style.bold,
                    });
                    styles(
                        run,
                        &range,
                        TextStyle {
                            bold: false,
                            ..*style
                        },
                    );
                    pad(run, Pad::Chip);
                }
                // Custom emoji wait for the emoji subsystem: the shortcode
                // shows as written.
                Inline::CustomEmoji { raw, .. } => {
                    push(run, raw);
                }
            }
        }
    }
}

/// The first text of `nodes`, if it opens them.
fn first_run(nodes: &mut [Node]) -> Option<&mut Run> {
    match nodes.first_mut()? {
        Node::Paragraph(run) | Node::Heading { run, .. } | Node::Code { run, .. } => Some(run),
        Node::Quote(nodes) => first_run(nodes),
        Node::List(items) => first_run(&mut items.first_mut()?.nodes),
        Node::Table { header, .. } => header.first_mut(),
        Node::Rule | Node::Blank | Node::Jumbo(_) => None,
    }
}

fn push(run: &mut Run, text: &str) -> Range<usize> {
    let start = run.text.len();
    run.text.push_str(text);
    start..run.text.len()
}

fn pad(run: &mut Run, pad_pt: Pad) {
    let range = push(run, PAD);
    run.marks.push(Mark::Pad { range, pad_pt });
}

fn styles(run: &mut Run, range: &Range<usize>, style: TextStyle) {
    if range.is_empty() {
        return;
    }
    if style.bold {
        run.marks.push(Mark::Bold(range.clone()));
    }
    if style.italic {
        run.marks.push(Mark::Italic(range.clone()));
    }
    if style.strikethrough {
        run.marks.push(Mark::Strike(range.clone()));
    }
}

/// "@Marina", "@everyone", or none for a person the account does not know.
fn mention_name(target: MentionTarget, resources: Option<&ResourceSnapshot>) -> Option<String> {
    match target {
        MentionTarget::Everyone => Some("@everyone".to_owned()),
        MentionTarget::User(id) => {
            let user = resources?.users.get(id)?;
            let name = user.nickname.as_ref().or(user.username.as_ref())?;
            Some(format!("@{name}"))
        }
    }
}
