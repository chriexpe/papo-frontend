//! Every committed icon file is one Rune draws as a neutral icon: the
//! expected grid, `currentColor` as its only ink, and nothing but plain
//! shapes (no script, link, style or reference to anything outside it).

use std::path::{Path, PathBuf};

/// The SVG namespace, the only `xmlns` a file may declare.
const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";
/// The elements the set draws with, which Rune's icon parser reads.
const ELEMENTS: [&str; 4] = ["svg", "path", "circle", "rect"];
/// The attributes those elements may carry: geometry and paint.
const ATTRIBUTES: [&str; 17] = [
    "xmlns",
    "viewBox",
    "d",
    "cx",
    "cy",
    "r",
    "x",
    "y",
    "width",
    "height",
    "rx",
    "fill",
    "fill-rule",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
];
/// The only values `fill` and `stroke` may take: no colour of its own.
const INKS: [&str; 2] = ["none", "currentColor"];

/// The committed SVG files in `assets/icons/{folder}`, sorted.
fn files(folder: &str) -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets/icons")
        .join(folder);
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{}: {error}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    files
}

/// One element's tag: its name and its `name="value"` attributes.
struct Tag<'a> {
    name: &'a str,
    attributes: Vec<(&'a str, &'a str)>,
}

/// The opening tags of `svg`, in order. Panics on anything but elements
/// with double-quoted attributes, closing tags and whitespace between them.
fn tags(svg: &str) -> Vec<Tag<'_>> {
    let mut tags = Vec::new();
    let mut rest = svg.trim();
    while !rest.is_empty() {
        let after_open = rest
            .strip_prefix('<')
            .unwrap_or_else(|| panic!("text outside a tag: {rest:.40}"));
        let end = after_open.find('>').expect("an unclosed tag");
        let inside = &after_open[..end];
        rest = after_open[end + 1..].trim_start();
        if inside.starts_with('/') {
            continue;
        }
        let inside = inside.strip_suffix('/').unwrap_or(inside);
        let name_end = inside.find(char::is_whitespace).unwrap_or(inside.len());
        tags.push(Tag {
            name: &inside[..name_end],
            attributes: attributes(&inside[name_end..]),
        });
    }
    tags
}

/// The `name="value"` pairs of one tag's attribute text.
fn attributes(mut text: &str) -> Vec<(&str, &str)> {
    let mut pairs = Vec::new();
    loop {
        text = text.trim_start();
        if text.is_empty() {
            return pairs;
        }
        let (name, after_name) = text
            .split_once("=\"")
            .unwrap_or_else(|| panic!("an attribute without a quoted value: {text:.40}"));
        let (value, after_value) = after_name.split_once('"').expect("an unclosed value");
        pairs.push((name, value));
        text = after_value;
    }
}

/// Checks one file: its root is the one `<svg>`, on a `view` × `view` grid,
/// it uses only the set's elements and attributes, and its ink is
/// `currentColor`.
fn check(path: &Path, view: &str) {
    let name = path.display();
    assert_eq!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("svg"),
        "{name}: only SVG files belong here"
    );
    let svg = std::fs::read_to_string(path).unwrap();
    let tags = tags(&svg);
    let root = tags.first().unwrap_or_else(|| panic!("{name} is empty"));
    assert_eq!(root.name, "svg", "{name}: the root is not <svg>");
    assert!(
        root.attributes
            .contains(&("viewBox", &format!("0 0 {view} {view}"))),
        "{name}: not on a {view} × {view} grid"
    );
    assert!(tags.len() > 1, "{name}: draws nothing inside its <svg>");
    assert!(
        tags[1..].iter().all(|tag| tag.name != "svg"),
        "{name}: an <svg> inside the root"
    );
    assert!(
        svg.trim_end().ends_with("</svg>"),
        "{name}: something after the root"
    );
    let mut inked = false;
    for tag in &tags {
        assert!(
            ELEMENTS.contains(&tag.name),
            "{name}: <{}> is not an element of the set",
            tag.name
        );
        for &(attribute, value) in &tag.attributes {
            assert!(
                ATTRIBUTES.contains(&attribute),
                "{name}: {attribute} is not an attribute of the set"
            );
            match attribute {
                "xmlns" => assert_eq!(value, SVG_NAMESPACE, "{name}"),
                "fill" | "stroke" => {
                    assert!(INKS.contains(&value), "{name}: {attribute}=\"{value}\"");
                    inked |= value == "currentColor";
                }
                _ => {}
            }
        }
    }
    assert!(inked, "{name}: nothing draws in currentColor");
}

#[test]
fn every_line_icon_is_a_neutral_24_unit_drawing() {
    let files = files("Icons");
    assert!(files.len() > 60, "the line icons: {}", files.len());
    for path in files {
        check(&path, "24");
        // §13's pen: 1.75 wide with round caps and joins, set on the root
        // so every shape inherits it.
        let svg = std::fs::read_to_string(&path).unwrap();
        let root = &tags(&svg)[0].attributes;
        for pen in [
            ("stroke-width", "1.75"),
            ("stroke-linecap", "round"),
            ("stroke-linejoin", "round"),
        ] {
            assert!(root.contains(&pen), "{}: {pen:?}", path.display());
        }
    }
}

#[test]
fn every_presence_mark_is_a_neutral_10_unit_drawing() {
    let files = files("Presence");
    assert_eq!(files.len(), 4, "online, away, busy and offline");
    for path in files {
        check(&path, "10");
    }
}
