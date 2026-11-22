//! Message content in the timeline (`docs/design.md` §9): a spoiler's
//! reveal moves nothing, code stops at 20 lines, a wide table scrolls inside
//! its message, one to three emoji draw large, and Ctrl+C copies the text as
//! shown, with headers across messages.

use std::sync::Arc;

use imbl::Vector;
use papo_sdk::{Message, Uuid};
use papo_sdk_ui::TimelineItem;
use papo_sdk_ui::message_content::MessageContent;
use rune::clipboard::memory;
use rune::{Key, KeyEvent, Mods, Runtime};

use super::{Host, assert_near, find_part, key, message, part, runtime, until_idle};
use crate::design::{
    CODE_BLOCK_MAX_LINES, Density, MessageLayout, TYPE_BODY, TYPE_CODE_BLOCK, TYPE_JUMBO,
};
use crate::timeline::TimelineHost;
use crate::timeline::content::Press;
use crate::timeline::rows::anchor_key;

fn normal() -> MessageLayout {
    MessageLayout::default()
}

/// Selects all of message `n` and copies it with Ctrl+C.
fn copy_message(rt: &mut Runtime<Host>, n: u128) -> String {
    copy(rt, &anchor_key(Uuid::from_u128(n)))
}

fn copy(rt: &mut Runtime<Host>, key: &str) -> String {
    let clipboard = memory();
    rt.use_clipboard(&clipboard);
    let key = key.to_owned();
    rt.update(move |_, tx| tx.select_all_text(key));
    until_idle(rt);
    rt.key(KeyEvent {
        key: Key::Command('c'),
        mods: Mods {
            cmd: true,
            ctrl: true,
            ..Mods::default()
        },
    });
    until_idle(rt);
    clipboard
        .text()
        .map(|text| text.to_string())
        .unwrap_or_default()
}

#[test]
fn revealing_a_spoiler_moves_nothing() {
    let items = Vector::from(vec![
        message(
            1,
            "ana",
            0,
            "the end is that ||the butler did it, with **the candlestick**, in the \
             library, and nobody saw a thing until the morning came|| and that is all",
        ),
        message(2, "bia", 60, "no way"),
    ]);
    let mut rt = runtime(items, normal());
    let before = (part(&rt, 1, "message"), part(&rt, 2, "message"));
    let id = Uuid::from_u128(1);
    rt.update(move |host, tx| host.content_pressed(key(), id, Press::Spoiler(0), tx));
    until_idle(&mut rt);
    assert_eq!(rt.state.open.revealed(id), &[0]);
    let after = (part(&rt, 1, "message"), part(&rt, 2, "message"));
    assert_eq!(before, after, "the rows keep their size and place");
}

#[test]
fn a_hidden_spoiler_copies_as_its_markup_and_a_revealed_one_as_its_text() {
    let items = Vector::from(vec![message(
        1,
        "ana",
        0,
        "it was ||the butler|| all along",
    )]);
    let mut rt = runtime(items, normal());
    assert_eq!(copy_message(&mut rt, 1), "it was ||the butler|| all along");
    let id = Uuid::from_u128(1);
    rt.update(move |host, tx| host.content_pressed(key(), id, Press::Spoiler(0), tx));
    until_idle(&mut rt);
    assert_eq!(copy_message(&mut rt, 1), "it was the butler all along");
}

#[test]
fn a_copy_takes_the_text_as_drawn() {
    let body = "**Plan** for `deploy`:\n\n\
                1. build\n2. ship it\n\n\
                - [x] tests\n- [ ] docs\n\n\
                > quoted\n\n\
                | a | b |\n|---|--:|\n| 1 | 2 |\n\n\
                ```rust\nfn main() {}\n```";
    let items = Vector::from(vec![message(1, "ana", 0, body)]);
    let mut rt = runtime(items, normal());
    assert_eq!(
        copy_message(&mut rt, 1),
        "Plan for deploy:\n1. build\n2. ship it\n☑ tests\n☐ docs\nquoted\na\tb\n1\t2\nfn main() {}"
    );
}

#[test]
fn a_copy_across_messages_heads_each_with_its_author_and_time() {
    let yesterday = Arc::new(TimelineItem::Message {
        reply: None,
        pending: None,
        id: Uuid::from_u128(1),
        message: Arc::new(Message {
            id: Some(Uuid::from_u128(1).to_string()),
            author_id: Some("ana".to_owned()),
            content: Some("first".to_owned()),
            created_at: Some("2026-11-19T09:04:00Z".to_owned()),
            ..Message::default()
        }),
        content: Arc::new(MessageContent::parse("first")),
    });
    let items = Vector::from(vec![yesterday, message(2, "bia", 0, "second")]);
    let mut rt = runtime(items, normal());
    assert_eq!(
        copy(&mut rt, "rows"),
        "Someone — 19 November, 09:04\nfirst\n\nSomeone — 14:00\nsecond"
    );
}

#[test]
fn a_long_code_block_stops_at_twenty_lines_and_scrolls_inside() {
    let code: Vec<String> = (0..30)
        .map(|n| format!("let line_{n} = \"{}\";", "x".repeat(120)))
        .collect();
    let body = format!("```rust\n{}\n```", code.join("\n"));
    let items = Vector::from(vec![message(1, "ana", 0, &body)]);
    let rt = runtime(items, normal());
    let lines = part(&rt, 1, "lines");
    assert_near(
        lines[3],
        TYPE_CODE_BLOCK.line_height * CODE_BLOCK_MAX_LINES,
        "the code's height",
    );
    let body = part(&rt, 1, "body");
    assert!(lines[2] <= body[2] + 0.5, "lines never widen the message");
}

#[test]
fn a_wide_table_scrolls_inside_its_message() {
    let cell = "a cell that holds a good many words";
    let body = format!(
        "| {cell} | {cell} | {cell} | {cell} |\n|---|---|---|---|\n| {cell} | {cell} | {cell} | {cell} |"
    );
    let items = Vector::from(vec![message(1, "ana", 0, &body)]);
    let rt = runtime(items, normal());
    let table = part(&rt, 1, "b0");
    let body = part(&rt, 1, "body");
    assert_near(table[2], body[2], "the table keeps to the message");
    let cells: f32 = (0..4)
        .map(|column| part(&rt, 1, &format!("r0/c{column}"))[2])
        .sum();
    assert!(cells > body[2], "the columns keep their width: {cells}");
}

#[test]
fn a_tables_header_fill_ends_with_its_last_column() {
    let body = "| a | b |\n|---|---|\n| 1 | 2 |";
    let items = Vector::from(vec![message(1, "ana", 0, body)]);
    let rt = runtime(items, normal());
    let header = find_part(&rt, 1, "r0").expect("the header row");
    assert_eq!(
        header.fill.a, 0.0,
        "the row, as wide as the message, has no fill"
    );
    let cell = find_part(&rt, 1, "r0/c1").expect("a header cell");
    assert!(cell.fill.a > 0.0, "the cells carry the header's fill");
}

#[test]
fn one_to_three_emoji_draw_large_but_not_at_extra_compact() {
    let items = || Vector::from(vec![message(1, "ana", 0, "🎉🔥")]);
    // The system's emoji font may stand a little taller than the line it
    // sits on, so the heights are bounds.
    let rt = runtime(items(), normal());
    let jumbo = part(&rt, 1, "t0")[3];
    assert!(
        jumbo >= TYPE_JUMBO.line_height - 0.5,
        "a jumbo line: {jumbo}"
    );
    let compact = MessageLayout {
        density: Density::ExtraCompact,
        ..normal()
    };
    let rt = runtime(items(), compact);
    let line = part(&rt, 1, "t0")[3];
    assert!(line < TYPE_BODY.line_height * 1.5, "a text line: {line}");
}
