//! Mention, custom emoji, cross-layer, nesting bounds and plain-text detection tests.

use papo_sdk::{MentionTarget, contains_user_mention};

use super::tests::{ID_A, ID_B, blocks, only_paragraph, plain_text, uuid};
use super::*;

#[test]
fn mentions_and_custom_emojis_follow_markdown_aware_placement() {
    let alice = uuid(ID_A);
    let bob = uuid(ID_B);
    let msg = format!(
        "Hello **@mention(<@{ID_A}>)** and @MENTION(<@{}>) and @EVERYONE! \
         mail@everyone.test @everyonex :wave: `:wave:` [@everyone](https://example.test)",
        ID_B.to_ascii_uppercase()
    );
    let catalog = [CustomEmojiRef {
        id: alice,
        name: "wave".into(),
    }];
    let parsed = MessageContent::parse_with_custom_emojis(&msg, &catalog);
    let inlines = only_paragraph(&parsed.blocks);

    assert!(inlines.iter().any(|inline| matches!(
        inline,
        Inline::Mention {
            target: MentionTarget::User(id),
            raw,
            style,
        } if *id == alice && raw == &format!("@mention(<@{ID_A}>)") && style.bold
    )));
    assert!(inlines.iter().any(|inline| matches!(
        inline,
        Inline::Mention {
            target: MentionTarget::User(id),
            raw,
            style,
        } if *id == bob && raw.starts_with("@MENTION(<@") && !style.bold
    )));
    let everyones: Vec<_> = inlines
        .iter()
        .filter(|inline| {
            matches!(
                inline,
                Inline::Mention {
                    target: MentionTarget::Everyone,
                    ..
                }
            )
        })
        .collect();
    assert_eq!(everyones.len(), 1);
    assert!(
        inlines
            .iter()
            .any(|inline| matches!(inline, Inline::CustomEmoji { id, raw } if *id == alice && raw == ":wave:"))
    );
    assert!(
        inlines
            .iter()
            .any(|inline| matches!(inline, Inline::Code(code) if code == ":wave:"))
    );
}

#[test]
fn code_span_mention_renders_literally_while_classification_detects_it() {
    let target_id = uuid(ID_A);
    let wire = format!("Look at `@mention(<@{ID_A}>)` and\n```\n@mention(<@{ID_A}>)\n```");

    assert!(
        contains_user_mention(&wire, target_id),
        "Layer 1 notification classification must match canonical token inside code"
    );

    let rendered = MessageContent::parse(&wire);
    let has_mention_node = rendered.blocks.iter().any(|block| match block {
        Block::Paragraph(inlines) => inlines.iter().any(|i| matches!(i, Inline::Mention { .. })),
        _ => false,
    });
    assert!(
        !has_mention_node,
        "Layer 2 renderer must keep mentions in code literal"
    );
    assert!(matches!(
        &rendered.blocks[0],
        Block::Paragraph(inlines) if inlines.contains(&Inline::Code(format!("@mention(<@{ID_A}>)")))
    ));
    assert!(matches!(
        &rendered.blocks[1],
        Block::CodeBlock { text, .. } if text == &format!("@mention(<@{ID_A}>)")
    ));
}

#[test]
fn excessive_nesting_is_bounded_and_with_custom_emojis_does_not_reparse() {
    let nested_quotes = format!("{}kept", "> ".repeat(2000));
    let parsed_quotes = MessageContent::parse(&nested_quotes);
    assert!(parsed_quotes.max_nesting_depth() <= MAX_NESTING_DEPTH);
    assert!(plain_text(&parsed_quotes.blocks).contains("kept"));

    let nested_spoilers = format!("{}secret{}", "||".repeat(120), "||".repeat(120));
    let parsed_spoilers = MessageContent::parse(&nested_spoilers);
    assert!(parsed_spoilers.max_nesting_depth() <= MAX_NESTING_DEPTH);

    let wave_id = uuid(ID_A);
    let base = MessageContent::parse("hi :wave: `:wave:`");
    let resolved = base.with_custom_emojis(&[CustomEmojiRef {
        id: wave_id,
        name: "wave".into(),
    }]);
    let inlines = only_paragraph(&resolved.blocks);
    assert!(
        inlines
            .iter()
            .any(|inline| matches!(inline, Inline::CustomEmoji { id, .. } if *id == wave_id))
    );
    assert!(
        inlines
            .iter()
            .any(|inline| matches!(inline, Inline::Code(code) if code == ":wave:"))
    );
}

#[test]
fn plain_detection_matches_markdown_rules() {
    for plain in [
        "hello?",
        "10-20 and 3.5",
        "a\nb\n\nc",
        "hi :x: 👋",
        "https://a.com",
        "+ test",
        "+2",
    ] {
        assert!(is_plain(plain), "{plain:?} should be plain");
    }
    for formatted in [
        "**b**", "_i_", "~s~", "`c`", "\\*", "[a](b)", "# t", "- a", "1. a", "> q", "| a |",
        "    code", "a<b>", "&amp;", "![a](b)", "---", "===",
    ] {
        assert!(!is_plain(formatted), "{formatted:?} should require parser");
    }
}

#[test]
fn escaped_backslash_pairs_and_blank_marker_literal_do_not_corrupt_output() {
    assert_eq!(
        only_paragraph(&blocks("\\\\||secret||")),
        &[
            Inline::Text {
                text: "\\".into(),
                style: TextStyle::default(),
                link: None,
            },
            Inline::Spoiler(vec![Inline::Text {
                text: "secret".into(),
                style: TextStyle::default(),
                link: None,
            }]),
        ]
    );
    assert_eq!(
        only_paragraph(&blocks("a | b || c")),
        &[Inline::Text {
            text: "a | b || c".into(),
            style: TextStyle::default(),
            link: None,
        }]
    );
    assert_eq!(
        plain_text(&blocks("PAPOMESSAGEBLANKLINETOKEN\n\n\nafter")),
        "PAPOMESSAGEBLANKLINETOKENafter"
    );
}

#[test]
fn fences_and_blanks_preserve_alternate_code_blocks_lists_and_inline_backticks() {
    assert_eq!(
        blocks("~~~md\n```rust\nfn x(){}\n```\n~~~"),
        vec![Block::CodeBlock {
            info: Some("md".into()),
            text: "```rust\nfn x(){}\n```".into(),
        }]
    );
    assert_eq!(
        blocks("````md\n```inner\n```\n````"),
        vec![Block::CodeBlock {
            info: Some("md".into()),
            text: "```inner\n```".into(),
        }]
    );
    assert_eq!(
        only_paragraph(&blocks("`before ```x``` after`")),
        &[Inline::Code("before ```x``` after".into())]
    );
    assert_eq!(
        blocks("```rust,no_run\nfn main(){}```"),
        vec![Block::CodeBlock {
            info: Some("rust,no_run".into()),
            text: "fn main(){}".into(),
        }]
    );
    assert_eq!(
        blocks("```python foo=bar\nx=1\n```"),
        vec![Block::CodeBlock {
            info: Some("python foo=bar".into()),
            text: "x=1".into(),
        }]
    );
    assert_eq!(
        blocks("~~~\na\n\nb\n~~~"),
        vec![Block::CodeBlock {
            info: None,
            text: "a\n\nb".into(),
        }]
    );
    assert_eq!(
        blocks("    code 1\n\n    code 2"),
        vec![Block::CodeBlock {
            info: None,
            text: "code 1\n\ncode 2".into(),
        }]
    );
    assert!(matches!(
        blocks("- first\n\n- second").as_slice(),
        [Block::List { items, .. }] if items.len() == 2
    ));
    assert!(matches!(
        blocks("- para1\n\n  para2").as_slice(),
        [Block::List { items, .. }] if items.len() == 1 && items[0].blocks.len() == 2
    ));
}

#[test]
fn unsafe_links_images_and_html_blocks_keep_tokens_literal_and_flush_paragraphs() {
    let parsed_unsafe = blocks(&format!(
        "[||secret|| @everyone @mention(<@{ID_A}>)](javascript:alert(1))"
    ));
    let unsafe_inlines = only_paragraph(&parsed_unsafe);
    assert!(
        !unsafe_inlines
            .iter()
            .any(|inline| matches!(inline, Inline::Spoiler(_) | Inline::Mention { .. }))
    );

    let hello_unsafe = blocks("hello [@everyone](javascript:alert(1))");
    assert_eq!(
        only_paragraph(&hello_unsafe),
        &[Inline::Text {
            text: "hello @everyone".into(),
            style: TextStyle::default(),
            link: None,
        }]
    );

    let image_alt = blocks("![||secret|| @everyone](https://a.com/x.png)");
    assert_eq!(
        only_paragraph(&image_alt),
        &[Inline::Text {
            text: "||secret|| @everyone".into(),
            style: TextStyle::default(),
            link: Some("https://a.com/x.png".into()),
        }]
    );

    let html_split = blocks("- first\n  <!-- c -->\n  second");
    let Block::List { items, .. } = &html_split[0] else {
        panic!("expected list block, got {html_split:?}");
    };
    assert_eq!(items[0].blocks.len(), 2);
}
