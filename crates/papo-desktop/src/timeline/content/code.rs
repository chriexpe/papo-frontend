//! A code block (`docs/design.md` §9, Code blocks): JetBrains Mono on
//! `surface.code`, coloured by Rune's highlighter, its fence's language
//! above it, lines that never wrap, at most 20 lines tall before it scrolls
//! inside, and a Copy button that shows on hover or focus.

use std::cell::RefCell;

use rune::{El, Span, col, icon, rich, row, text};
use rune_highlights::{Highlighter, Lang, Token};

use super::Draw;
use super::doc::Run;
use super::inline::text_key;
use crate::design::{
    CHIP_ICON_PT, CODE_BLOCK_MAX_LINES, CODE_BLOCK_OUTLINE_PT, CODE_BLOCK_PAD_PT,
    CODE_BLOCK_RADIUS, CODE_LABEL_GAP, COPY_BUTTON_HEIGHT_PT, COPY_BUTTON_INSET_PT,
    COPY_BUTTON_PAD_PT, CodeInk, FOCUS_RING_PT, Look, SPACE_XS, TYPE_CAPTION_MEDIUM,
    TYPE_CODE_BLOCK, TYPE_SMALL,
};
use crate::icons::CHECK;
use crate::strings;
use crate::timeline::TimelineHost;

thread_local! {
    /// The highlighter's lexers and memory, reused from block to block.
    static HIGHLIGHTER: RefCell<Highlighter> = RefCell::new(Highlighter::new());
}

/// The code block whose code is `run`, fenced as `language`.
pub(super) fn block<S: TimelineHost>(language: Option<&str>, run: &Run, draw: &Draw<'_>) -> El {
    let look = draw.look;
    let spans = highlight(language, &run.text, look);
    let code = rich(run.text.clone(), spans)
        .key(text_key(run.index))
        .font(look.font(TYPE_CODE_BLOCK))
        .color(look.palette.text_primary)
        .selectable(true)
        .selection_color(look.palette.text_selection);
    let lines = col()
        .key("lines")
        .w_full()
        .max_h(look.font(TYPE_CODE_BLOCK).line_height * CODE_BLOCK_MAX_LINES)
        .scroll_xy()
        // The keyboard scrolls it once focused.
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .child(code);
    let mut inside = col().w_full().gap(look.space(CODE_LABEL_GAP));
    // A fence that names no language gets no label: the client never
    // guesses.
    if let Some(language) = language {
        inside = inside.child(
            text(language.to_owned())
                .key("language")
                .font(look.font(TYPE_SMALL))
                .color(look.palette.text_muted),
        );
    }
    let copied = draw.copied == Some(run.index);
    let (vertical, horizontal) = CODE_BLOCK_PAD_PT;
    col()
        .key("code")
        .w_full()
        .py(vertical)
        .px(horizontal)
        .radius(look.radius(CODE_BLOCK_RADIUS))
        .fill(look.palette.surface_code)
        .border(CODE_BLOCK_OUTLINE_PT, look.palette.line_subtle)
        .hover_group()
        .child(inside.child(lines))
        .child(copy_button::<S>(run, draw, copied))
}

/// "Copy" in the block's top-right corner, ✓ "Copied" for a moment after
/// a press.
fn copy_button<S: TimelineHost>(run: &Run, draw: &Draw<'_>, copied: bool) -> El {
    let look = draw.look;
    let (key, message, index) = (draw.key, draw.message, run.index);
    let code = run.text.clone();
    let label = if copied {
        strings::code_copied()
    } else {
        strings::code_copy()
    };
    let button = row()
        .key("copy")
        .pin(
            None,
            Some(COPY_BUTTON_INSET_PT),
            Some(COPY_BUTTON_INSET_PT),
            None,
        )
        .h(COPY_BUTTON_HEIGHT_PT)
        .px(COPY_BUTTON_PAD_PT)
        .center()
        // Concentric with the block, whose corner is the inset out.
        .radius(Look::nested_radius(
            look.radius(CODE_BLOCK_RADIUS),
            COPY_BUTTON_INSET_PT,
        ))
        .fill(look.palette.surface_pill)
        .border(CODE_BLOCK_OUTLINE_PT, look.palette.line_strong)
        .role("button")
        .label(strings::copy_code_label())
        .focusable(true)
        .focus_ring(FOCUS_RING_PT.0, FOCUS_RING_PT.1, look.palette.focus_ring)
        .on_press_tx(move |host: &mut S, tx| {
            tx.copy(code.clone());
            host.code_copied(key, message, index, tx);
        })
        .gap(look.space(SPACE_XS))
        .children(copied.then(|| icon(&CHECK, CHIP_ICON_PT).color(look.palette.text_secondary)))
        .child(
            text(label)
                .font(look.font(TYPE_CAPTION_MEDIUM))
                .color(look.palette.text_secondary),
        );
    if copied {
        button
    } else {
        button.group_opacity(0.0, 1.0)
    }
}

/// `code`'s token colours as `language` lexes it; none for a language the
/// highlighter does not know.
fn highlight(language: Option<&str>, code: &str, look: &Look) -> Vec<Span> {
    let Some(lang) = language.and_then(Lang::from_name) else {
        return Vec::new();
    };
    let tokens = HIGHLIGHTER.with_borrow_mut(|highlighter| highlighter.highlight(lang, code));
    tokens
        .into_iter()
        .filter_map(|(range, token)| {
            let colour = token_colour(token, &look.palette.code, look)?;
            Some(Span::new(range.start, range.end).color(colour))
        })
        .collect()
}

/// The colour of `token` (Zed's capture names); none leaves it
/// `text.primary`.
fn token_colour(token: Token, ink: &CodeInk, look: &Look) -> Option<rune::Color> {
    let name = token.name();
    let family = name.split('.').next().unwrap_or(name);
    match family {
        "keyword" => Some(ink.keyword),
        "string" => Some(ink.string),
        "number" | "constant" | "boolean" => Some(ink.number),
        "type" | "enum" | "variant" | "constructor" => Some(ink.type_name),
        "function" => Some(ink.function),
        "comment" => Some(look.palette.text_muted),
        _ => None,
    }
}
