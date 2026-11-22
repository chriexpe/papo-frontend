//! What one line says about a message, from synthetic messages.

use super::*;
use serde_json::{Value, json};

fn of(source: &str) -> Snippet {
    of_message(&Message {
        content: Some(source.to_owned()),
        ..Message::default()
    })
}

fn of_message(message: &Message) -> Snippet {
    let content = MessageContent::parse(message.content.as_deref().unwrap_or_default());
    snippet(message, &content)
}

fn text(value: &str) -> Snippet {
    Snippet::Text(vec![SnippetPart::Text(value.to_owned())])
}

/// A message from its wire JSON.
fn wire(value: Value) -> Message {
    serde_json::from_value(value).unwrap()
}

#[test]
fn the_first_line_of_text_without_its_markdown() {
    assert_eq!(
        of("**is** the _build_ green?\nsecond line"),
        text("is the build green?")
    );
    assert_eq!(of("\n\n  # Title  \n\nbody"), text("Title"));
    assert_eq!(of("> quoted\n\nafter"), text("quoted"));
    assert_eq!(of("- first\n- second"), text("first"));
    assert_eq!(
        of("```rust\nlet a = 1;\nlet b = 2;\n```"),
        text("let a = 1;")
    );
}

#[test]
fn mentions_and_spoilers_stay_parts_and_spoiler_text_never_shows() {
    let id = Uuid::from_u128(7);
    let Snippet::Text(parts) = of(&format!("hi @mention(<@{id}>) ||secret|| ok")) else {
        panic!("text");
    };
    assert_eq!(
        parts,
        vec![
            SnippetPart::Text("hi ".into()),
            SnippetPart::Mention(MentionTarget::User(id)),
            SnippetPart::Text(" ".into()),
            SnippetPart::Spoiler,
            SnippetPart::Text(" ok".into()),
        ]
    );
}

#[test]
fn without_text_the_first_attachment_names_the_kind() {
    for (mime, expected) in [
        ("image/gif", Snippet::Gif),
        ("image/png", Snippet::Image),
        ("video/mp4", Snippet::Video),
        (
            "application/pdf",
            Snippet::File {
                name: "notes.pdf".into(),
            },
        ),
    ] {
        let snippet = of_message(&wire(json!({
            "attachments": [{ "mime_type": mime, "original_file_name": "notes.pdf" }],
        })));
        assert_eq!(snippet, expected, "{mime}");
    }
    let captioned = of_message(&wire(json!({
        "content": "look",
        "attachments": [{ "mime_type": "image/png", "original_file_name": "a.png" }],
    })));
    assert_eq!(captioned, text("look"), "text comes first");
}

#[test]
fn a_link_only_message_is_its_embeds_title_and_host() {
    let embed = json!({ "title": "Papo release notes", "url": "https://example.org/notes" });
    let linked = of_message(&wire(json!({
        "content": "https://example.org/notes",
        "embeds": [embed.clone()],
    })));
    assert_eq!(
        linked,
        Snippet::Link {
            title: Some("Papo release notes".into()),
            domain: Some("example.org".into()),
        }
    );
    let untitled = of_message(&Message {
        content: Some("https://example.org/notes".into()),
        ..Message::default()
    });
    assert_eq!(
        untitled,
        Snippet::Link {
            title: None,
            domain: Some("example.org".into()),
        }
    );
    let embed_only = of_message(&wire(json!({ "embeds": [embed] })));
    assert!(matches!(embed_only, Snippet::Link { title: Some(_), .. }));
    assert!(matches!(
        of("see https://example.org/notes"),
        Snippet::Text(_)
    ));
}

#[test]
fn nothing_to_show_is_empty() {
    assert_eq!(of(""), Snippet::Empty);
    assert_eq!(of("---"), Snippet::Empty);
}
