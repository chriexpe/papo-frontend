//! Building a [`Doc`](super::Doc) from the SDK's blocks: each text numbered
//! in document order, each spoiler in the order a reader meets it, chips
//! padded, mentions named.

use std::ops::Range;

use papo_sdk::{MentionTarget, ResourceSnapshot};
use papo_sdk_ui::message_content::{Block, Inline, ListItem, TextStyle};

use super::{Context, Item, Joint, Mark, Marker, Node, PAD, Pad, Run};
use crate::strings;
use crate::timeline::content::emoji::JumboPart;

/// Numbers texts and spoilers as it lays blocks out.
pub(super) struct Builder<'a, 'c> {
    pub(super) context: &'a Context<'c>,
    pub(super) texts: usize,
    pub(super) spoilers: u32,
}

impl Builder<'_, '_> {
    pub(super) fn blocks(&mut self, blocks: &[Block], lists: usize) -> Vec<Node> {
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
    pub(super) fn plain(&mut self, text: String, joint: Joint) -> Run {
        let index = self.next_text();
        Run {
            index,
            text,
            marks: Vec::new(),
            prefix: None,
            joint,
            jumbo: false,
        }
    }

    /// A jumbo message's run: its Unicode emoji as text, its custom emoji marked.
    pub(super) fn jumbo(&mut self, parts: &[JumboPart<'_>]) -> Run {
        let mut run = self.plain(String::new(), Joint::Line);
        run.jumbo = true;
        for part in parts {
            match part {
                JumboPart::Text(text) => {
                    push(&mut run, text);
                }
                JumboPart::Custom { id, raw } => {
                    let range = push(&mut run, raw);
                    run.marks.push(Mark::Emoji { range, id: *id });
                }
            }
        }
        run
    }

    fn run(&mut self, inlines: &[Inline], joint: Joint) -> Run {
        let index = self.next_text();
        let mut run = Run {
            index,
            text: String::new(),
            marks: Vec::new(),
            prefix: None,
            joint,
            jumbo: false,
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
                    // Marked before what it holds, so an outer spoiler's press
                    // comes before an inner one's, as the reader meets them.
                    let mark = run.marks.len();
                    run.marks.push(Mark::Spoiler {
                        range: start..start,
                        index,
                        hidden: shut,
                    });
                    self.inlines(run, inner);
                    let end = run.text.len();
                    if let Some(Mark::Spoiler { range, .. }) = run.marks.get_mut(mark) {
                        range.end = end;
                    }
                    pad(run, Pad::Spoiler);
                }
                Inline::Mention { target, style, .. } => {
                    let name = mention_name(*target, self.context.resources);
                    pad(run, Pad::Chip);
                    let label = name.as_deref().unwrap_or_else(|| strings::unknown_person());
                    let range = push(run, label);
                    let person = match target {
                        MentionTarget::User(user) if name.is_some() => Some(*user),
                        _ => None,
                    };
                    run.marks.push(Mark::Mention {
                        range: range.clone(),
                        known: name.is_some(),
                        bold: style.bold,
                        person,
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
                // Its `:name:` stays the text, so a selection copies it; the
                // drawing sets the image over it.
                Inline::CustomEmoji { id, raw } => {
                    let range = push(run, raw);
                    run.marks.push(Mark::Emoji { range, id: *id });
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
pub(super) fn mention_name(
    target: MentionTarget,
    resources: Option<&ResourceSnapshot>,
) -> Option<String> {
    match target {
        MentionTarget::Everyone => Some("@everyone".to_owned()),
        MentionTarget::User(id) => {
            let user = resources?.users.get(id)?;
            let name = user.nickname.as_ref().or(user.username.as_ref())?;
            Some(format!("@{name}"))
        }
    }
}
