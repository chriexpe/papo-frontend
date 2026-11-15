//! Property tests ensuring arbitrary Markdown inputs stay safe, bounded and panic-free.

use proptest::prelude::*;

use super::*;

fn check_invariants(input: &str) {
    let parsed = MessageContent::parse(input);
    assert!(
        parsed.max_nesting_depth() <= MAX_NESTING_DEPTH,
        "depth {} exceeded {MAX_NESTING_DEPTH}",
        parsed.max_nesting_depth()
    );
    for block in &parsed.blocks {
        check_block_links(block);
    }

    let catalog = [CustomEmojiRef {
        id: papo_sdk::Uuid::from_u128(1),
        name: "wave".into(),
    }];
    assert_eq!(
        parsed.with_custom_emojis(&catalog),
        MessageContent::parse_with_custom_emojis(input, &catalog)
    );

    let spans = preview_spans(input);
    for span in spans {
        assert!(span.byte_range.start <= span.byte_range.end);
        assert!(span.byte_range.end <= input.len());
        assert!(input.is_char_boundary(span.byte_range.start));
        assert!(input.is_char_boundary(span.byte_range.end));
        assert!(span.utf16_range.start <= span.utf16_range.end);
    }
}

fn check_block_links(block: &Block) {
    match block {
        Block::Paragraph(inlines) | Block::Heading { inlines, .. } => {
            check_inline_links(inlines);
        }
        Block::BlockQuote(blocks) => {
            for inner in blocks {
                check_block_links(inner);
            }
        }
        Block::List { items, .. } => {
            for item in items {
                for inner in &item.blocks {
                    check_block_links(inner);
                }
            }
        }
        Block::Table { header, rows, .. } => {
            for cell in header {
                check_inline_links(cell);
            }
            for row in rows {
                for cell in row {
                    check_inline_links(cell);
                }
            }
        }
        Block::CodeBlock { .. } | Block::ThematicBreak | Block::BlankLine => {}
    }
}

fn check_inline_links(inlines: &[Inline]) {
    for inline in inlines {
        match inline {
            Inline::Text {
                link: Some(url), ..
            } => {
                assert!(is_safe_href(url), "unsafe href leaked into tree: {url:?}");
            }
            Inline::Spoiler(inner) => check_inline_links(inner),
            _ => {}
        }
    }
}

proptest! {
    #[test]
    fn arbitrary_unicode_and_delimiter_soup_never_panic_and_stay_bounded(
        arbitrary in "\\PC{0,512}",
        soup in prop::collection::vec(
            prop_oneof![
                Just("*"), Just("**"), Just("_"), Just("~"), Just("`"), Just("```"),
                Just("|"), Just("||"), Just("\\"), Just("> "), Just("- "), Just("+ "),
                Just("["), Just("]"), Just("("), Just(")"), Just("<script>"),
                Just("javascript:alert(1)"), Just(":wave:"), Just("!"),
                Just("@mention(<@11111111-2222-3333-4444-555555555555>)"),
                Just("@everyone"), Just("\n"), Just("\r\n"), Just("a"),
            ],
            0..120
        )
    ) {
        check_invariants(&arbitrary);
        let joined = soup.concat();
        check_invariants(&joined);
    }

    #[test]
    fn mention_encode_decode_round_trips(
        prefix in "[\\PC&&[^@]]{0,24}",
        mid in "[\\PC&&[^@]]{0,24}",
        suffix in "[\\PC&&[^@]]{0,24}",
        raw_uuid_a in any::<u128>(),
        raw_uuid_b in any::<u128>(),
    ) {
        let id_a = papo_sdk::Uuid::from_u128(raw_uuid_a);
        let id_b = papo_sdk::Uuid::from_u128(raw_uuid_b);
        let wire = format!(
            "{prefix}{} {mid} {}{suffix}",
            papo_sdk::encode_user_mention(id_a),
            papo_sdk::encode_user_mention(id_b),
        );
        let decoded = decode_composer_mentions(&wire, |id| {
            if id == id_a { Some("Alpha") } else { None }
        });
        let round_tripped = encode_composer_mentions(&decoded.text, &decoded.bindings);
        prop_assert_eq!(round_tripped, wire);
    }
}
