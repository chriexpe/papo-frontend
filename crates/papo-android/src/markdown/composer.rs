use papo_core::markdown::composer::{self, Kind};

#[derive(Clone, Debug, uniffi::Record)]
pub struct ComposerMarkdownSpan {
    pub start: u32,
    pub end: u32,
    pub kind: String,
    pub level: u32,
}

/// Compose/Android editor offsets are UTF-16, unlike the parser's UTF-8 byte ranges.
#[uniffi::export]
pub fn preview_composer_markdown(source: String) -> Vec<ComposerMarkdownSpan> {
    let styles = composer::styles(&source);
    if styles.is_empty() {
        return Vec::new();
    }
    let mut offsets = vec![0u32; source.len() + 1];
    let mut utf16 = 0u32;
    for (byte, ch) in source.char_indices() {
        offsets[byte] = utf16;
        utf16 += ch.len_utf16() as u32;
    }
    offsets[source.len()] = utf16;
    styles
        .into_iter()
        .map(|span| {
            let (kind, level) = match span.kind {
                Kind::Heading(level) => ("heading", u32::from(level)),
                Kind::Bold => ("bold", 0),
                Kind::Italic => ("italic", 0),
                Kind::Strike => ("strike", 0),
                Kind::Code => ("code", 0),
                Kind::CodeBlock => ("code-block", 0),
                Kind::Link => ("link", 0),
                Kind::Quote => ("quote", 0),
            };
            ComposerMarkdownSpan {
                start: offsets[span.range.start],
                end: offsets[span.range.end],
                kind: kind.into(),
                level,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_offsets_match_android_utf16_without_rewriting_source() {
        let source = "🙂 **café** and *日本語*";
        let spans = preview_composer_markdown(source.into());
        let bold = spans.iter().find(|span| span.kind == "bold").unwrap();
        assert_eq!((bold.start, bold.end), (3, 11));
        let italic = spans.iter().find(|span| span.kind == "italic").unwrap();
        assert_eq!((italic.start, italic.end), (16, 21));
        let heading = preview_composer_markdown("## Test\nbody".into());
        assert_eq!(heading[0].level, 2);
        assert_eq!((heading[0].start, heading[0].end), (0, 7));
    }
}
