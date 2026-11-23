//! Markers and content found inside the SDK's real preview spans.

use papo_sdk_ui::message_content::{PreviewSpanKind, preview_spans};

use super::{field_style, parts};
use crate::design::{Look, Theme};

/// For each span of `kind` in `source`: its content and its markers, as text.
fn split(source: &str, kind: PreviewSpanKind) -> Vec<(String, Vec<String>)> {
    preview_spans(source)
        .into_iter()
        .filter(|span| span.kind == kind)
        .map(|span| {
            let parts = parts(source, span.kind, span.byte_range).unwrap();
            (
                source[parts.content].to_owned(),
                parts
                    .markers
                    .into_iter()
                    .map(|marker| source[marker].to_owned())
                    .collect(),
            )
        })
        .collect()
}

fn one(content: &str, markers: &[&str]) -> Vec<(String, Vec<String>)> {
    vec![(
        content.to_owned(),
        markers.iter().map(|&marker| marker.to_owned()).collect(),
    )]
}

#[test]
fn emphasis_dims_its_delimiters() {
    assert_eq!(
        split("a **oi** b", PreviewSpanKind::Bold),
        one("oi", &["**", "**"])
    );
    assert_eq!(
        split("a *oi* b", PreviewSpanKind::Italic),
        one("oi", &["*", "*"])
    );
    assert_eq!(
        split("a _oi_ b", PreviewSpanKind::Italic),
        one("oi", &["_", "_"])
    );
    assert_eq!(
        split("a ~~oi~~ b", PreviewSpanKind::Strikethrough),
        one("oi", &["~~", "~~"])
    );
    assert_eq!(
        split("a ||oi|| b", PreviewSpanKind::Spoiler),
        one("oi", &["||", "||"])
    );
}

#[test]
fn code_dims_however_many_backticks_open_it() {
    assert_eq!(
        split("a `x` b", PreviewSpanKind::Code),
        one("x", &["`", "`"])
    );
    assert_eq!(
        split("a ``x`y`` b", PreviewSpanKind::Code),
        one("x`y", &["``", "``"])
    );
}

#[test]
fn a_heading_dims_its_hashes_and_the_space() {
    assert_eq!(
        split("## Título\nbody", PreviewSpanKind::Heading(2)),
        one("Título", &["## "])
    );
}

#[test]
fn a_link_shows_its_label_and_dims_the_rest() {
    assert_eq!(
        split(
            "see [the guide](https://example.org/a) now",
            PreviewSpanKind::Link
        ),
        one("the guide", &["[", "](https://example.org/a)"])
    );
}

#[test]
fn a_quote_dims_each_line_s_markers() {
    let quoted = split("> one\n> > two", PreviewSpanKind::BlockQuote);
    let markers: Vec<&str> = quoted[0].1.iter().map(String::as_str).collect();
    assert_eq!(markers, ["> ", "> ", "> "]);
}

#[test]
fn a_fence_dims_its_lines_and_keeps_the_code() {
    assert_eq!(
        split("```rust\nlet a = 1;\n```", PreviewSpanKind::CodeBlock),
        one("let a = 1;", &["```rust", "```"])
    );
}

#[test]
fn a_fence_still_open_is_all_marker_until_the_code_starts() {
    let open = split("```rust", PreviewSpanKind::CodeBlock);
    if let Some((content, markers)) = open.first() {
        assert_eq!(content, "");
        assert_eq!(markers, &["```rust"]);
    }
}

#[test]
fn markers_come_after_the_content_so_their_dimming_wins() {
    let look = Look::new(Theme::Graphite);
    let source = "**oi**";
    let style = field_style(source, &preview_spans(source), &look);
    let last = style.spans.last().unwrap();
    assert_eq!(last.color, Some(look.palette.text_muted));
    assert_eq!(style.spans[0].weight, Some(crate::design::WEIGHT_SEMIBOLD));
    assert_eq!((style.spans[0].start, style.spans[0].end), (2, 4));
}

#[test]
fn a_strikethrough_draws_a_line_over_its_content_only() {
    let look = Look::new(Theme::Graphite);
    let source = "~~oi~~";
    let style = field_style(source, &preview_spans(source), &look);
    assert_eq!(style.strikes, [(2, 4, None)]);
}

#[test]
fn no_style_changes_the_text_s_size() {
    let look = Look::new(Theme::Graphite);
    let source = "# h\n**b** *i* `c` ||s|| [l](https://example.org) > q\n```\nx\n```";
    let style = field_style(source, &preview_spans(source), &look);
    assert!(style.spans.iter().all(|span| span.size.is_none()));
}
