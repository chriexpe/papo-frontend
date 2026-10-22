//! Flat, typed presentation tree. Parsing and formatting rules live in core;
//! Compose only lays out nodes. Parse before publishing a timeline snapshot so
//! rich text cannot change row geometry after the reading reducer sees it.
use papo_core::{
    markdown::{self, Block, ColumnAlign, Inline},
    state::Store,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone, Debug, Default, Deserialize, Serialize, uniffi::Record)]
pub struct MarkdownDocument {
    pub source: String,
    pub roots: Vec<u32>,
    pub nodes: Vec<MarkdownNode>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, uniffi::Record)]
pub struct MarkdownNode {
    pub kind: String,
    pub level: u32,
    pub start: Option<u64>,
    pub task: Option<bool>,
    pub language: String,
    pub text: String,
    pub spans: Vec<MarkdownSpan>,
    pub children: Vec<u32>,
    pub alignments: Vec<String>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, uniffi::Record)]
pub struct MarkdownSpan {
    pub kind: String,
    pub text: String,
    pub link: String,
    pub user_id: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
}
impl MarkdownDocument {
    fn node(&mut self, node: MarkdownNode) -> u32 {
        let index = self.nodes.len() as u32;
        self.nodes.push(node);
        index
    }
    fn blocks(&mut self, blocks: &[Block]) -> Vec<u32> {
        blocks
            .iter()
            .map(|block| {
                let mut node = MarkdownNode::default();
                match block {
                    Block::Paragraph(spans) => {
                        node.kind = "paragraph".into();
                        node.spans = spans.iter().map(span).collect();
                    }
                    Block::Heading(level, spans) => {
                        node.kind = "heading".into();
                        node.level = u32::from(*level);
                        node.spans = spans.iter().map(span).collect();
                    }
                    Block::Quote(blocks) => {
                        node.kind = "quote".into();
                        node.children = self.blocks(blocks);
                    }
                    Block::List { start, items } => {
                        node.kind = "list".into();
                        node.start = *start;
                        node.children = items
                            .iter()
                            .map(|item| {
                                let children = self.blocks(&item.blocks);
                                self.node(MarkdownNode {
                                    kind: "item".into(),
                                    task: item.task,
                                    children,
                                    ..Default::default()
                                })
                            })
                            .collect();
                    }
                    Block::Code { lang, text } => {
                        node.kind = "code".into();
                        node.language = lang.clone().unwrap_or_default();
                        node.text = text.clone();
                    }
                    Block::Rule => node.kind = "rule".into(),
                    Block::Spacer => node.kind = "spacer".into(),
                    Block::Table { aligns, head, rows } => {
                        node.kind = "table".into();
                        node.alignments = aligns
                            .iter()
                            .map(|a| {
                                match a {
                                    ColumnAlign::Left => "left",
                                    ColumnAlign::Center => "center",
                                    ColumnAlign::Right => "right",
                                }
                                .into()
                            })
                            .collect();
                        node.children = std::iter::once(head)
                            .chain(rows)
                            .map(|row| {
                                let children = row
                                    .iter()
                                    .map(|cell| {
                                        self.node(MarkdownNode {
                                            kind: "cell".into(),
                                            spans: cell.iter().map(span).collect(),
                                            ..Default::default()
                                        })
                                    })
                                    .collect();
                                self.node(MarkdownNode {
                                    kind: "row".into(),
                                    children,
                                    ..Default::default()
                                })
                            })
                            .collect();
                    }
                }
                self.node(node)
            })
            .collect()
    }
}
fn span(value: &Inline) -> MarkdownSpan {
    let mut result = MarkdownSpan::default();
    match value {
        Inline::Text { text, style, link } => {
            result.kind = "text".into();
            result.text = text.clone();
            result.link = link.clone().unwrap_or_default();
            result.bold = style.bold;
            result.italic = style.italic;
            result.strike = style.strike;
        }
        Inline::Code(text) => {
            result.kind = "code".into();
            result.text = text.clone();
        }
        Inline::Image { src, alt } => {
            result.kind = "image".into();
            result.link = src.clone();
            result.text = alt.clone();
        }
        Inline::Mention { user_id, label } => {
            result.kind = "mention".into();
            result.user_id = user_id.clone();
            result.text = format!("@{label}");
        }
        Inline::Break => {
            result.kind = "break".into();
            result.text = "\n".into();
        }
    }
    result
}
fn build(source: &str, prepared: &markdown::Prepared) -> MarkdownDocument {
    let mut doc = MarkdownDocument {
        source: source.into(),
        ..Default::default()
    };
    doc.roots = doc.blocks(&markdown::parse(prepared));
    doc
}
type Documents = VecDeque<(String, Arc<MarkdownDocument>, usize)>;
/// Account and resolved mention labels are part of the key. Bound both entry
/// count and estimated memory; a single huge message is never retained.
pub(super) fn document(store: &Store, server: &str, source: &str) -> Arc<MarkdownDocument> {
    static CACHE: OnceLock<Mutex<Documents>> = OnceLock::new();
    let (display, bindings) = store.display_mentions_with_bindings(source);
    let prepared = markdown::substitute_mentions(&display, &bindings);
    let key = format!("{server}\0{}\0{}\0{source}", store.me, prepared.key());
    let cache = CACHE.get_or_init(Default::default);
    if let Ok(mut cache) = cache.lock()
        && let Some(at) = cache.iter().position(|(k, _, _)| k == &key)
    {
        let entry = cache.remove(at).expect("index found");
        let doc = entry.1.clone();
        cache.push_back(entry);
        return doc;
    }
    let doc = Arc::new(build(source, &prepared));
    let bytes = key.len() + serde_json::to_vec(doc.as_ref()).map_or(0, |v| v.len());
    if bytes <= 4 * 1024 * 1024
        && let Ok(mut cache) = cache.lock()
    {
        while cache.len() >= 256
            || cache.iter().map(|e| e.2).sum::<usize>() + bytes > 4 * 1024 * 1024
        {
            cache.pop_front();
        }
        cache.push_back((key, doc.clone(), bytes));
    }
    doc
}

/// Also used by previews and tests without a timeline Store. Real message
/// snapshots always use account-resolved mention bindings above.
#[uniffi::export]
pub fn parse_message_markdown(source: String) -> MarkdownDocument {
    build(&source, &markdown::substitute_mentions(&source, &[]))
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct FormatEdit {
    pub text: String,
    pub caret: u32,
}
impl From<papo_core::format_assist::Edit> for FormatEdit {
    fn from(edit: papo_core::format_assist::Edit) -> Self {
        Self {
            text: edit.text,
            caret: edit.caret as u32,
        }
    }
}
/// One ephemeral session per composer. No runtime or transport locks involved.
#[derive(uniffi::Object, Default)]
pub struct FormatAssistant {
    pairing: Mutex<Option<papo_core::format_assist::Pairing>>,
}
#[uniffi::export]
impl FormatAssistant {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    pub fn reset(&self) {
        if let Ok(mut pairing) = self.pairing.lock() {
            *pairing = None;
        }
    }
    pub fn change(&self, previous: String, text: String, caret: u32) -> Option<FormatEdit> {
        let mut pairing = self.pairing.lock().ok()?;
        papo_core::format_assist::on_change(&previous, &text, caret as usize, &mut pairing)
            .map(Into::into)
    }
    pub fn track(&self, previous: String, text: String) {
        if let Ok(mut pairing) = self.pairing.lock() {
            papo_core::format_assist::track_change(&previous, &text, &mut pairing);
        }
    }
    pub fn skip(&self, text: String, caret: u32) -> Option<FormatEdit> {
        let mut pairing = self.pairing.lock().ok()?;
        papo_core::format_assist::skip_closer(&text, caret as usize, &mut pairing).map(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cached_documents_follow_resolved_names_and_account_identity() {
        let mut demo = crate::demo::Demo::new();
        let source = "**Hello** @mention(<@ana>)";
        let first = document(&demo.store, "https://example.test", source);
        assert!(
            first
                .nodes
                .iter()
                .flat_map(|n| &n.spans)
                .any(|s| s.kind == "mention" && s.text == "@Ana" && s.user_id == "ana")
        );
        assert!(Arc::ptr_eq(
            &first,
            &document(&demo.store, "https://example.test", source)
        ));
        demo.store
            .members
            .iter_mut()
            .find(|m| m.id == "ana")
            .unwrap()
            .name = "a_*name".into();
        let renamed = document(&demo.store, "https://example.test", source);
        assert!(!Arc::ptr_eq(&first, &renamed));
        assert!(
            renamed
                .nodes
                .iter()
                .flat_map(|n| &n.spans)
                .any(|s| s.kind == "mention" && s.text == "@a_*name" && s.user_id == "ana")
        );
        demo.store.me = "another-account".into();
        assert!(!Arc::ptr_eq(
            &renamed,
            &document(&demo.store, "https://example.test", source)
        ));
    }
    #[test]
    fn flat_tree_references_children_and_preserves_code_literals() {
        let doc = parse_message_markdown(
            "> - [x] **done**\n\n```rust\nlet url = \"https://cdn.test/code.gif\";\n```".into(),
        );
        assert!(
            doc.roots
                .iter()
                .chain(doc.nodes.iter().flat_map(|n| &n.children))
                .all(|i| (*i as usize) < doc.nodes.len())
        );
        assert!(doc.nodes.iter().any(|n| n.task == Some(true)));
        assert!(doc.nodes.iter().any(|n| n.kind == "code"
            && n.text.contains("https://cdn.test/code.gif")
            && n.spans.is_empty()));
    }
}
