//! Laying content out and copying it back, without a view: chips' padding
//! never copies, hidden spoilers copy as their markup, mentions name people,
//! list items carry their markers.

use std::ops::Range;

use papo_sdk::Uuid;
use papo_sdk_ui::message_content::{Block, Inline, MessageContent, TextStyle};

use super::Press;
use super::copy::{run_text, spoken};
use super::doc::{Context, Doc, Mark, Marker, Node, Run};

fn doc(source: &str, revealed: &[u32]) -> Doc {
    Doc::new(
        &MessageContent::parse(source),
        &Context {
            resources: None,
            revealed,
            jumbo: true,
        },
    )
}

fn whole(run: &Run) -> String {
    run_text(run, 0..run.text.len())
}

#[test]
fn presses_survive_the_trip_through_a_link_id() {
    for press in [
        Press::Url("https://example.com/a#b".to_owned()),
        Press::Url("mailto:lia@example.com".to_owned()),
        Press::Spoiler(7),
        Press::Edited,
    ] {
        assert_eq!(Press::parse(&press.id()), Some(press));
    }
}

#[test]
fn chips_padding_never_copies() {
    let doc = doc("run `cargo test` now", &[]);
    let runs = doc.runs();
    assert_ne!(runs[0].text, "run cargo test now", "the chip is padded");
    assert_eq!(whole(runs[0]), "run cargo test now");
}

#[test]
fn a_hidden_spoiler_copies_whole_even_when_half_selected() {
    let doc = doc("a ||secret|| b", &[]);
    let run = doc.runs()[0];
    assert_eq!(whole(run), "a ||secret|| b");
    let inside = run.text.find("cret").expect("the spoiler's words");
    assert_eq!(run_text(run, inside..run.text.len()), "||secret|| b");
    let revealed = doc_revealed("a ||secret|| b");
    assert_eq!(whole(revealed.runs()[0]), "a secret b");
}

fn doc_revealed(source: &str) -> Doc {
    doc(source, &[0])
}

#[test]
fn assistive_technology_hears_a_hidden_spoiler_announced_not_read() {
    let hidden = doc("ends ||the butler|| and `code`", &[]);
    assert_eq!(
        spoken(hidden.runs()[0]).as_deref(),
        Some("ends Hidden spoiler. Press to reveal. and code")
    );
    let revealed = doc("ends ||the butler|| and `code`", &[0]);
    assert_eq!(
        spoken(revealed.runs()[0]).as_deref(),
        Some("ends the butler and code")
    );
    assert_eq!(spoken(doc("plain words", &[]).runs()[0]), None);
}

#[test]
fn a_spoiler_inside_a_hidden_one_copies_inside_it_and_comes_second() {
    // The SDK's parser does not nest spoilers today, but its model can.
    let text = |words: &str| Inline::Text {
        text: words.to_owned(),
        style: TextStyle::default(),
        link: None,
        allow_tokens: true,
    };
    let content = MessageContent {
        blocks: vec![Block::Paragraph(vec![Inline::Spoiler(vec![
            text("a "),
            Inline::Spoiler(vec![text("b")]),
            text(" c"),
        ])])],
    };
    let doc = Doc::new(
        &content,
        &Context {
            resources: None,
            revealed: &[],
            jumbo: true,
        },
    );
    let run = doc.runs()[0];
    let spoilers: Vec<(u32, Range<usize>)> = run
        .marks
        .iter()
        .filter_map(|mark| match mark {
            Mark::Spoiler { index, range, .. } => Some((*index, range.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(spoilers.len(), 2, "two spoilers, one inside the other");
    let (outer, inner) = (&spoilers[0].1, &spoilers[1].1);
    assert!(outer.start < inner.start && inner.end < outer.end);
    assert_eq!(spoilers[0].0, 0, "the outer is met first");
    assert_eq!(whole(run), "||a b c||");
}

#[test]
fn a_mention_the_server_does_not_know_is_unknown_never_its_token() {
    let id = Uuid::from_u128(5);
    let source = format!("hi {}", papo_sdk::mentions::encode_user_mention(id));
    let doc = doc(&source, &[]);
    let run = doc.runs()[0];
    assert_eq!(whole(run), "hi @unknown");
    assert!(
        run.marks
            .iter()
            .any(|mark| matches!(mark, Mark::Mention { known: false, .. }))
    );
    let everyone = self::doc("@everyone look", &[]);
    assert_eq!(whole(everyone.runs()[0]), "@everyone look");
}

#[test]
fn numbers_share_the_widest_numbers_column() {
    let source: String = (1..=10).map(|n| n.to_string() + ". item\n").collect();
    let doc = doc(&source, &[]);
    let Some(Node::List(items)) = doc.nodes.first() else {
        panic!("a list");
    };
    assert_eq!(
        items[8].marker,
        Marker::Number { value: 9, width: 2 },
        "9 sits in a column as wide as 10"
    );
    assert_eq!(doc.runs()[9].prefix.as_deref(), Some("10. "));
}

#[test]
fn edited_follows_text_but_not_a_block() {
    assert!(doc("words", &[]).ends_in_text());
    assert!(!doc("```\ncode\n```", &[]).ends_in_text());
    assert!(!doc("- item", &[]).ends_in_text());
}
