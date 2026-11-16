//! Unit tests for pure composer helpers: preview spans, selection format, pair closing and mentions.

use super::*;
use papo_sdk::{MentionTarget, Uuid};

const ID_A: &str = "11111111-2222-3333-4444-555555555555";
const ID_UNKNOWN: &str = "99999999-8888-7777-6666-555555555555";

#[test]
fn preview_spans_keep_utf8_and_utf16_offsets() {
    let source = "🙂 **café** and *日本語* ||secret||";
    let spans = preview_spans(source);

    let bold = spans
        .iter()
        .find(|s| s.kind == PreviewSpanKind::Bold)
        .unwrap();
    assert_eq!(&source[bold.byte_range.clone()], "**café**");
    assert_eq!(bold.utf16_range, 3..11);

    let italic = spans
        .iter()
        .find(|s| s.kind == PreviewSpanKind::Italic)
        .unwrap();
    assert_eq!(&source[italic.byte_range.clone()], "*日本語*");
    assert_eq!(italic.utf16_range, 16..21);

    let spoiler = spans
        .iter()
        .find(|s| s.kind == PreviewSpanKind::Spoiler)
        .unwrap();
    assert_eq!(&source[spoiler.byte_range.clone()], "||secret||");

    let heading = preview_spans("## Test\nbody");
    assert_eq!(heading[0].kind, PreviewSpanKind::Heading(2));
    assert_eq!(heading[0].byte_range, 0..7);
    assert_eq!(heading[0].utf16_range, 0..7);
}

#[test]
fn preview_spans_handle_code_unsafe_links_mentions_and_bounds() {
    let id = Uuid::parse_str(ID_A).unwrap();
    let mention = format!("@mention(<@{ID_A}>)");
    let source = format!("```rust\n**lit**\n```\n[bad](javascript:alert) {mention}");
    let spans = preview_spans(&source);
    assert!(spans.iter().any(|s| s.kind == PreviewSpanKind::CodeBlock));
    assert!(!spans.iter().any(|s| s.kind == PreviewSpanKind::Bold));
    assert!(!spans.iter().any(|s| s.kind == PreviewSpanKind::Link));
    assert!(spans.iter().any(|s| {
        s.kind == PreviewSpanKind::Mention(MentionTarget::User(id))
            && source[s.byte_range.clone()] == mention
    }));

    assert_eq!(preview_spans(&format!("{}kept", "> ".repeat(2000))), []);
    assert_eq!(preview_spans(&"**many** ".repeat(2050)), []);
    assert_eq!(preview_spans(&"x".repeat(128 * 1024 + 1)), []);

    let display = "Hi @A*B! *it*";
    let masked = preview_spans_with_bindings(
        display,
        &[ComposerMentionBinding {
            id,
            byte_range: 3..8,
            utf16_range: 3..8,
            text: "@A*B!".into(),
        }],
    );
    let italic = masked
        .iter()
        .find(|s| s.kind == PreviewSpanKind::Italic)
        .unwrap();
    assert_eq!(&display[italic.byte_range.clone()], "*it*");
}

#[test]
fn selection_format_wraps_and_unwraps_every_kind() {
    let text = "hello world";
    for (format, wrapped) in [
        (SelectionFormat::Bold, "hello **world**"),
        (SelectionFormat::Italic, "hello *world*"),
        (SelectionFormat::Strikethrough, "hello ~~world~~"),
        (SelectionFormat::Code, "hello `world`"),
        (SelectionFormat::Spoiler, "hello ||world||"),
    ] {
        let edit = apply_selection_format(text, 6..11, &format).unwrap();
        assert_eq!(edit.text, wrapped);
        let unwrapped_inner =
            apply_selection_format(&edit.text, edit.byte_selection.clone(), &format).unwrap();
        assert_eq!(unwrapped_inner.text, text);
        assert_eq!(unwrapped_inner.byte_selection, 6..11);

        let outer_end = edit.byte_selection.end + (wrapped.len() - text.len()) / 2;
        let outer_start = edit.byte_selection.start - (wrapped.len() - text.len()) / 2;
        let unwrapped_outer =
            apply_selection_format(&edit.text, outer_start..outer_end, &format).unwrap();
        assert_eq!(unwrapped_outer.text, text);
        assert_eq!(unwrapped_outer.byte_selection, 6..11);
    }

    let bold_italic = apply_selection_format("**hi**", 2..4, &SelectionFormat::Italic).unwrap();
    assert_eq!(bold_italic.text, "***hi***");
    let back_to_bold = apply_selection_format(
        &bold_italic.text,
        bold_italic.byte_selection,
        &SelectionFormat::Italic,
    )
    .unwrap();
    assert_eq!(back_to_bold.text, "**hi**");

    let multi = apply_selection_format("a\nb", 0..3, &SelectionFormat::Code).unwrap();
    assert_eq!(multi.text, "```\na\nb\n```");
    assert_eq!(
        apply_selection_format(&multi.text, multi.byte_selection, &SelectionFormat::Code)
            .unwrap()
            .text,
        "a\nb"
    );

    let linked = apply_selection_format(
        "site",
        0..4,
        &SelectionFormat::Link {
            url: Some("https://a.test".into()),
        },
    )
    .unwrap();
    assert_eq!(linked.text, "[site](https://a.test)");
    assert_eq!(
        apply_selection_format(
            &linked.text,
            linked.byte_selection,
            &SelectionFormat::Link { url: None }
        )
        .unwrap()
        .text,
        "site"
    );
    let empty_link =
        apply_selection_format("site", 0..4, &SelectionFormat::Link { url: None }).unwrap();
    assert_eq!(empty_link.text, "[site]()");
    assert_eq!(empty_link.byte_selection, 7..7);

    assert_eq!(
        apply_selection_format("é", 1..2, &SelectionFormat::Bold),
        Err(InvalidSelection)
    );

    let wiki = "[Rust](https://en.wikipedia.org/wiki/Rust_(programming_language)) tail";
    let unwrap_whole = apply_selection_format(
        wiki,
        0..(wiki.len() - " tail".len()),
        &SelectionFormat::Link { url: None },
    )
    .unwrap();
    assert_eq!(unwrap_whole.text, "Rust tail");
    let unwrap_label =
        apply_selection_format(wiki, 1..5, &SelectionFormat::Link { url: None }).unwrap();
    assert_eq!(unwrap_label.text, "Rust tail");
    assert_eq!(unwrap_label.byte_selection, 0..4);
}

fn type_into(
    text: &mut String,
    byte_caret: &mut usize,
    pairing: &mut Option<PairClosing>,
    input: &str,
) {
    for ch in input.chars() {
        let prev = text.clone();
        text.insert(*byte_caret, ch);
        *byte_caret += ch.len_utf8();
        if let Some(edit) =
            on_pair_change(&prev, text, *byte_caret, TextChangeSource::Typing, pairing)
        {
            *text = edit.text;
            *byte_caret = edit.byte_caret;
        }
    }
}

#[test]
fn pair_closing_handles_markdown_backticks_and_brackets_on_typing() {
    let (mut t, mut c, mut p) = (String::new(), 0, None);
    type_into(&mut t, &mut c, &mut p, "**oi");
    assert_eq!((t.as_str(), c), ("**oi**", 4));
    type_into(&mut t, &mut c, &mut p, "**");
    assert_eq!((t.as_str(), c), ("**oi** ", 7));

    let (mut t, mut c, mut p) = (String::new(), 0, None);
    type_into(&mut t, &mut c, &mut p, "```x");
    assert_eq!(t, "```x\n```");
    let skipped = skip_pair_closer(&t, c, &mut p).unwrap();
    assert_eq!(skipped.text, "```x\n```");

    let (mut t, mut c, mut p) = (String::new(), 0, None);
    type_into(&mut t, &mut c, &mut p, "([");
    assert_eq!((t.as_str(), c), ("([])", 2));
    type_into(&mut t, &mut c, &mut p, "x])");
    assert_eq!((t.as_str(), c), ("([x])", 5));
    assert!(p.is_none());

    let (mut t, mut c, mut p) = (String::new(), 0, None);
    type_into(&mut t, &mut c, &mut p, "{a");
    assert_eq!(t, "{a}");
    let skipped = skip_pair_closer(&t, c, &mut p).unwrap();
    assert_eq!((skipped.text.as_str(), skipped.byte_caret), ("{a}", 3));
}

#[test]
fn pair_closing_never_fires_on_paste_or_ime_commit() {
    for pasted in ["`", "```", "(", "[", "{", "**", "__", "~~", "||"] {
        let mut pairing = None;
        assert!(
            on_pair_change(
                "",
                pasted,
                pasted.len(),
                TextChangeSource::Paste,
                &mut pairing
            )
            .is_none()
        );
        assert!(pairing.is_none());

        let mut ime_pairing = None;
        assert!(
            on_pair_change(
                "",
                pasted,
                pasted.len(),
                TextChangeSource::ImeCommit,
                &mut ime_pairing,
            )
            .is_none()
        );
        assert!(ime_pairing.is_none());
    }

    // Active pairing shifts cleanly across IME/paste without inserting duplicate closers.
    let (mut t, mut c, mut p) = (String::new(), 0, None);
    type_into(&mut t, &mut c, &mut p, "**");
    assert!(on_pair_change(&t, "**café**", 7, TextChangeSource::ImeCommit, &mut p).is_none());
    let jumped = skip_pair_closer("**café**", 7, &mut p).unwrap();
    assert_eq!(jumped.text, "**café** ");
    assert_eq!(jumped.utf16_caret, 9);
}

#[test]
fn composer_mentions_round_trip_and_preserve_unknown_ids_with_utf16_offsets() {
    let known = Uuid::parse_str(ID_A).unwrap();
    let unknown_wire = format!("@mention(<@{ID_UNKNOWN}>)");
    let wire = format!("🙂 @mention(<@{ID_A}>) and {unknown_wire}");

    let decoded = decode_composer_mentions(&wire, |id| (id == known).then_some("Alice"));
    assert_eq!(decoded.text, format!("🙂 @Alice and {unknown_wire}"));
    assert_eq!(decoded.bindings.len(), 1);
    assert_eq!(decoded.bindings[0].byte_range, 5..11);
    assert_eq!(decoded.bindings[0].utf16_range, 3..9);
    assert_eq!(
        utf16_range_to_utf8(&decoded.text, &(3..9)),
        Some(decoded.bindings[0].byte_range.clone())
    );

    let reencoded = encode_composer_mentions(&decoded.text, &decoded.bindings);
    assert_eq!(reencoded, wire);
}

#[test]
fn preview_and_pair_closing_edge_cases_stay_sound() {
    let id = Uuid::parse_str(ID_A).unwrap();
    assert_eq!(
        preview_spans(&format!("{}kept @everyone ||sec||", "> ".repeat(2000))),
        []
    );
    assert!(
        !preview_spans("- ||a\n- b||")
            .iter()
            .any(|s| s.kind == PreviewSpanKind::Spoiler)
    );
    assert!(
        !preview_spans("![||a|| @everyone](https://a.invalid/x.png)")
            .iter()
            .any(|s| matches!(
                s.kind,
                PreviewSpanKind::Spoiler | PreviewSpanKind::Mention(_)
            ))
    );

    let dedup_text = "hi @everyone";
    let dedup_spans = preview_spans_with_bindings(
        dedup_text,
        &[ComposerMentionBinding {
            id,
            byte_range: 3..12,
            utf16_range: 3..12,
            text: "@everyone".into(),
        }],
    );
    let mention_kinds: Vec<_> = dedup_spans
        .iter()
        .filter_map(|s| match s.kind {
            PreviewSpanKind::Mention(target) => Some(target),
            _ => None,
        })
        .collect();
    assert_eq!(mention_kinds, vec![MentionTarget::User(id)]);

    // Nested pair exit right before an outer active closer must not insert a space before `)`.
    let (mut t, mut c, mut p) = (String::new(), 0, None);
    type_into(&mut t, &mut c, &mut p, "(**oi");
    assert_eq!(t, "(**oi**)");
    type_into(&mut t, &mut c, &mut p, "**");
    assert_eq!((t.as_str(), c), ("(**oi**)", 7));

    // Inserting before frame.start keeps active pairing aligned and intact.
    let (mut t, mut c, mut p) = (String::new(), 0, None);
    type_into(&mut t, &mut c, &mut p, "**oi");
    let shifted = "x**oi**";
    assert!(on_pair_change(&t, shifted, 1, TextChangeSource::Typing, &mut p).is_none());
    let jumped = skip_pair_closer(shifted, 5, &mut p).unwrap();
    assert_eq!(jumped.text, "x**oi** ");
}
