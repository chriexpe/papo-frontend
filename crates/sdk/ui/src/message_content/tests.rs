//! Parser, sanitization, spoiler, nesting and Layer 2 mention/emoji tests.

use super::*;
use papo_sdk::Uuid;

pub(super) const ID_A: &str = "11111111-2222-3333-4444-555555555555";
pub(super) const ID_B: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";

pub(super) fn uuid(raw: &str) -> Uuid {
    Uuid::parse_str(raw).expect("valid test UUID")
}

pub(super) fn blocks(text: &str) -> Vec<Block> {
    MessageContent::parse(text).into_blocks()
}

impl MessageContent {
    fn into_blocks(self) -> Vec<Block> {
        self.blocks
    }
}

pub(super) fn only_paragraph(list: &[Block]) -> &[Inline] {
    match list {
        [Block::Paragraph(inlines)] => inlines,
        other => panic!("expected single paragraph, got {other:?}"),
    }
}

pub(super) fn plain_text(list: &[Block]) -> String {
    fn walk_inlines(inlines: &[Inline], out: &mut String) {
        for inline in inlines {
            match inline {
                Inline::Text { text, .. } | Inline::Code(text) => out.push_str(text),
                Inline::LineBreak => out.push('\n'),
                Inline::Spoiler(inner) => walk_inlines(inner, out),
                Inline::Mention { raw, .. } | Inline::CustomEmoji { raw, .. } => out.push_str(raw),
            }
        }
    }
    fn walk_blocks(blocks: &[Block], out: &mut String) {
        for block in blocks {
            match block {
                Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
                    walk_inlines(inlines, out);
                }
                Block::BlockQuote(inner) => walk_blocks(inner, out),
                Block::List { items, .. } => {
                    for item in items {
                        walk_blocks(&item.blocks, out);
                    }
                }
                Block::CodeBlock { text, .. } => out.push_str(text),
                Block::Table { header, rows, .. } => {
                    for cell in header {
                        walk_inlines(cell, out);
                    }
                    for row in rows {
                        for cell in row {
                            walk_inlines(cell, out);
                        }
                    }
                }
                Block::ThematicBreak | Block::BlankLine => {}
            }
        }
    }
    let mut out = String::new();
    walk_blocks(list, &mut out);
    out
}

fn text_runs(inlines: &[Inline]) -> Vec<(&str, TextStyle, Option<&str>)> {
    inlines
        .iter()
        .filter_map(|inline| match inline {
            Inline::Text {
                text, style, link, ..
            } => Some((text.as_str(), *style, link.as_deref())),
            _ => None,
        })
        .collect()
}

fn blank_lines(list: &[Block]) -> usize {
    list.iter()
        .filter(|block| matches!(block, Block::BlankLine))
        .count()
}

#[test]
fn empty_content_is_empty() {
    assert!(MessageContent::parse("").is_empty());
}

#[test]
fn plain_text_is_one_paragraph() {
    assert_eq!(
        only_paragraph(&blocks("hello")),
        &[Inline::Text {
            text: "hello".into(),
            style: TextStyle::default(),
            link: None,
            allow_tokens: true,
        }]
    );
}

#[test]
fn raw_html_is_dropped() {
    assert_eq!(blocks("<script>alert(1)</script>"), []);
    let parsed = blocks("a <b>bold</b> & c <d>e</d>");
    let text = plain_text(&parsed);
    assert!(!text.contains('<'));
    assert!(text.contains("a bold"));
    assert!(text.contains('&'));
    assert!(text.contains('e'));
}

#[test]
fn bold_italic_strikethrough_and_nested_styles() {
    let styled = blocks("**b** *i* ~~s~~ _u_ ~t~ __d__");
    let runs = text_runs(only_paragraph(&styled));
    assert!(runs.iter().any(|(t, s, _)| *t == "b" && s.bold));
    assert!(runs.iter().any(|(t, s, _)| *t == "i" && s.italic));
    assert!(runs.iter().any(|(t, s, _)| *t == "s" && s.strikethrough));
    assert!(runs.iter().any(|(t, s, _)| *t == "u" && s.italic));
    assert!(runs.iter().any(|(t, s, _)| *t == "t" && s.strikethrough));
    assert!(runs.iter().any(|(t, s, _)| *t == "d" && s.bold));

    let both = blocks("***both***");
    let nested = text_runs(only_paragraph(&both));
    assert!(
        nested
            .iter()
            .any(|(t, s, _)| *t == "both" && s.bold && s.italic)
    );
}

#[test]
fn single_newline_is_line_break_and_blank_lines_do_not_collapse() {
    assert!(only_paragraph(&blocks("a\nb")).contains(&Inline::LineBreak));

    let two = blocks("a\n\nb");
    assert_eq!(blank_lines(&two), 1);
    assert_eq!(plain_text(&two), "ab");
    assert!(matches!(two[0], Block::Paragraph(_)));
    assert!(matches!(two[1], Block::BlankLine));
    assert!(matches!(two[2], Block::Paragraph(_)));

    assert_eq!(blank_lines(&blocks("a\n\n\nb")), 2);
    assert_eq!(blank_lines(&blocks("a\n\n\n\nb")), 3);
    assert_eq!(blank_lines(&blocks("a\r\n\r\nb")), 1);

    let with_marker_literal = blocks("a\n\nPAPOMESSAGEBLANKLINETOKEN\n\nb");
    assert_eq!(blank_lines(&with_marker_literal), 2);
    assert!(plain_text(&with_marker_literal).contains("PAPOMESSAGEBLANKLINETOKEN"));
}

#[test]
fn fenced_and_inline_code_keep_contents_literal_and_preserve_info_string() {
    assert_eq!(
        blocks("```\na\n\nb\n```"),
        vec![Block::CodeBlock {
            info: None,
            text: "a\n\nb".into(),
        }]
    );
    assert_eq!(
        blocks("```\nfoo <bar> **x** :y: ||z||\n```"),
        vec![Block::CodeBlock {
            info: None,
            text: "foo <bar> **x** :y: ||z||".into(),
        }]
    );
    assert_eq!(
        blocks("```{\"ok\":true}```"),
        vec![Block::CodeBlock {
            info: None,
            text: "{\"ok\":true}".into(),
        }]
    );
    assert_eq!(
        blocks("```\nfirst\nsecond```"),
        vec![Block::CodeBlock {
            info: None,
            text: "first\nsecond".into(),
        }]
    );
    let glued = blocks("before```x```after");
    assert_eq!(glued.len(), 3);
    assert_eq!(
        glued[1],
        Block::CodeBlock {
            info: None,
            text: "x".into(),
        }
    );
    assert_eq!(
        blocks("```json\n{}\n```"),
        vec![Block::CodeBlock {
            info: Some("json".into()),
            text: "{}".into(),
        }]
    );
    assert_eq!(
        normalize_fenced_code_blocks("```not a lang\nbody```"),
        "```\nnot a lang\nbody\n```"
    );
    assert_eq!(
        normalize_fenced_code_blocks("```c++\nx```"),
        "```c++\nx\n```"
    );
    assert_eq!(
        normalize_fenced_code_blocks("a```b```c"),
        "a\n```\nb\n```\nc"
    );
    assert_eq!(normalize_fenced_code_blocks("```unclosed"), "```unclosed");

    let inline_blocks = blocks("before `x :y: ||z||` after");
    let inline = only_paragraph(&inline_blocks);
    assert!(inline.contains(&Inline::Code("x :y: ||z||".into())));
}

#[test]
fn headings_lists_quotes_rules_tasks_and_tables() {
    for level in 1..=6u8 {
        let parsed = blocks(&format!("{} title", "#".repeat(usize::from(level))));
        assert!(matches!(&parsed[0], Block::Heading { level: got, .. } if *got == level));
    }

    let doc = blocks("# T\n- a\n- b\n\n> q\n\n***\n\n| c | d |\n|:--|:--:|\n| 1 | 2 |");
    assert!(matches!(&doc[0], Block::Heading { level: 1, .. }));
    assert!(
        doc.iter()
            .any(|b| matches!(b, Block::List { items, .. } if items.len() == 2))
    );
    assert!(doc.iter().any(|b| matches!(b, Block::BlockQuote(_))));
    assert!(doc.contains(&Block::ThematicBreak));
    let Some(Block::Table {
        alignments,
        header,
        rows,
    }) = doc.last()
    else {
        panic!("expected table");
    };
    assert_eq!(alignments, &[TableAlignment::Left, TableAlignment::Center]);
    assert_eq!(header.len(), 2);
    assert_eq!(rows.len(), 1);

    let tasks = blocks("3. a\n4. b\n   - c\n\n- [x] done\n- [ ] todo");
    assert!(matches!(&tasks[0], Block::List { start: Some(3), .. }));
    let checks: Vec<bool> = tasks
        .iter()
        .filter_map(|b| match b {
            Block::List { items, .. } => Some(items),
            _ => None,
        })
        .flatten()
        .filter_map(|item| item.task)
        .collect();
    assert_eq!(checks, vec![true, false]);

    let aligned = blocks("| a | b | c | d |\n|:--|:-:|--:|---|\n| 1 | 2 | 3 | 4 |");
    let Some(Block::Table { alignments, .. }) = aligned.last() else {
        panic!("expected table");
    };
    assert_eq!(
        alignments,
        &[
            TableAlignment::Left,
            TableAlignment::Center,
            TableAlignment::Right,
            TableAlignment::Left,
        ]
    );
}

#[test]
fn leading_plus_stays_literal_without_breaking_code_blocks() {
    for content in ["+ test", "+", "+\ttest", "+ first\n+ second"] {
        let parsed = blocks(content);
        only_paragraph(&parsed);
        assert_eq!(plain_text(&parsed), content);
    }
    for code in ["```\n+ test\n```", "~~~\n+ test\n~~~", "    + test"] {
        assert!(
            matches!(blocks(code).as_slice(), [Block::CodeBlock { text, .. }] if text == "+ test")
        );
    }
    assert_eq!(
        only_paragraph(&blocks("`+ test`")),
        &[Inline::Code("+ test".into())]
    );
}

#[test]
fn links_allow_only_http_https_and_mailto_and_no_media_nodes_exist() {
    assert!(is_safe_href("https://a.com"));
    assert!(is_safe_href("HTTP://a.com"));
    assert!(is_safe_href("mailto:a@b.c"));
    assert!(!is_safe_href("#anchor"));
    assert!(!is_safe_href("/path"));
    assert!(!is_safe_href(""));
    assert!(!is_safe_href("javascript:alert(1)"));
    assert!(!is_safe_href("data:text/html,x"));
    assert!(!is_safe_href("ftp://a.com"));
    assert!(!is_safe_href("no-scheme.com"));

    let parsed = blocks(
        "[x](javascript:alert(1)) [y](vbscript:msg) [z](data:text/html,1) \
         [w](/relative) [h](#sec) [ok](https://e.com) [m](mailto:a@b.c) <https://auto.test>",
    );
    let runs = text_runs(only_paragraph(&parsed));
    assert!(plain_text(&parsed).starts_with("x y z w h ok"));
    for unsafe_label in ["x", "y", "z", "w", "h"] {
        assert!(
            runs.iter()
                .any(|(t, _, l)| *t == unsafe_label && l.is_none()),
            "missing literal run for {unsafe_label}: {runs:?}"
        );
    }
    assert!(
        runs.iter()
            .any(|(t, _, l)| *t == "ok" && *l == Some("https://e.com"))
    );
    assert!(
        runs.iter()
            .any(|(t, _, l)| *t == "m" && *l == Some("mailto:a@b.c"))
    );
    assert!(
        runs.iter()
            .any(|(t, _, l)| *t == "https://auto.test" && *l == Some("https://auto.test"))
    );

    let img_fallback = blocks("![cat](https://e.com/cat.png) ![bad](javascript:alert(1))");
    assert_eq!(plain_text(&img_fallback), "cat bad");
}

#[test]
fn escapes_entities_and_spoilers() {
    assert_eq!(plain_text(&blocks("&#65; \\*not bold\\*")), "A *not bold*");
    assert_eq!(
        only_paragraph(&blocks("||secret||")),
        &[Inline::Spoiler(vec![Inline::Text {
            text: "secret".into(),
            style: TextStyle::default(),
            link: None,
            allow_tokens: true,
        }])]
    );
    assert_eq!(
        only_paragraph(&blocks("||**bold**||")),
        &[Inline::Spoiler(vec![Inline::Text {
            text: "bold".into(),
            style: TextStyle {
                bold: true,
                italic: false,
                strikethrough: false,
            },
            link: None,
            allow_tokens: true,
        }])]
    );
    assert_eq!(
        only_paragraph(&blocks("\\||literal||")),
        &[Inline::Text {
            text: "||literal||".into(),
            style: TextStyle::default(),
            link: None,
            allow_tokens: true,
        }]
    );
    assert_eq!(
        only_paragraph(&blocks("||a \\| b||")),
        &[Inline::Spoiler(vec![Inline::Text {
            text: "a | b".into(),
            style: TextStyle::default(),
            link: None,
            allow_tokens: true,
        }])]
    );
    assert_eq!(
        only_paragraph(&blocks("||||")),
        &[Inline::Text {
            text: "||||".into(),
            style: TextStyle::default(),
            link: None,
            allow_tokens: true,
        }]
    );
    assert_eq!(
        only_paragraph(&blocks("||`code || pipe`||")),
        &[Inline::Spoiler(vec![Inline::Code("code || pipe".into())])]
    );
}
