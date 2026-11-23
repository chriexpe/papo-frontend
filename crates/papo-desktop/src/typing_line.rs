//! The typing line (`docs/design.md` §10): who else is typing in the shown
//! conversation, on one line over the composer at the text pill's text
//! inset. It overlays the timeline's bottom padding, so nothing moves when
//! it comes or goes, and it rises with the composer. Three dots pulse in
//! turn, still under reduced motion. Readable by assistive technology,
//! never announced: a live region would talk over the conversation.
//!
//! Owns the drawing; who types is the SDK's (`AccountActivity::typing_in`),
//! the wording `strings::typing_line`'s.

use papo_sdk::ResourceSnapshot;
use papo_sdk_ui::TypingItem;
use rune::{El, LoopSpec, Span, el, rich, row};

use crate::design::{
    PILL_OUTLINE_PT, Palette, SPACE_LG_PT, SPACE_SM_PT, TYPE_SMALL, TYPING_DOT_PT, TYPING_LINE_PT,
    TYPING_PULSE_S, WEIGHT_SEMIBOLD,
};
use crate::strings;

/// The line for `typing`, pinned above the composer column it is a child
/// of; none while nobody else types.
pub(crate) fn view(
    typing: &[TypingItem],
    resources: Option<&ResourceSnapshot>,
    palette: &Palette,
) -> Option<El> {
    let names: Vec<String> = typing
        .iter()
        .map(|typist| name(typist, resources))
        .collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    // A personal phrase is for one typer only; they don't combine.
    let phrase = match typing {
        [one] => one.phrase.as_deref(),
        _ => None,
    };
    let (line, ranges) = strings::typing_line(&names, phrase);
    if line.is_empty() {
        return None;
    }
    let spans = ranges
        .into_iter()
        .map(|range| {
            Span::new(range.start, range.end)
                .weight(WEIGHT_SEMIBOLD)
                .color(palette.text_secondary)
        })
        .collect();
    // At the pill's text: the column's padding, the pill's outline, then
    // the text's own inset.
    let inset_pt = SPACE_LG_PT + PILL_OUTLINE_PT + SPACE_LG_PT;
    Some(
        row()
            .key("typing-line")
            .h(TYPING_LINE_PT)
            .gap(SPACE_SM_PT)
            .center_y()
            .pin(
                Some(inset_pt),
                Some(SPACE_LG_PT - TYPING_LINE_PT),
                None,
                None,
            )
            .child(dots(palette))
            .child(
                rich(line, spans)
                    .font(TYPE_SMALL)
                    .color(palette.text_muted)
                    .max_lines(1),
            ),
    )
}

/// A typist's name: their live or known nickname, else their username.
fn name(typist: &TypingItem, resources: Option<&ResourceSnapshot>) -> String {
    let user = resources.and_then(|resources| resources.users.get(typist.user));
    typist
        .nickname
        .clone()
        .or_else(|| user.and_then(|user| user.nickname.clone()))
        .or_else(|| user.and_then(|user| user.username.clone()))
        .unwrap_or_else(|| strings::someone().to_owned())
}

/// Three dots, each pulsing a third of a period after the one before.
fn dots(palette: &Palette) -> El {
    let dot = |index: u32| {
        el().size(TYPING_DOT_PT, TYPING_DOT_PT)
            .radius(TYPING_DOT_PT / 2.0)
            .fill(palette.text_muted)
            .looping(LoopSpec::opacity(
                TYPING_PULSE_S,
                TYPING_PULSE_S * f64::from(index) / 3.0,
                0.3,
                1.0,
            ))
    };
    row()
        .gap(TYPING_DOT_PT / 2.0)
        .a11y_hidden()
        .children((0..3).map(dot))
}
