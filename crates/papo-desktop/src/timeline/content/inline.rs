//! A laid-out text ([`Run`]) drawn inline, as one selectable rich text: its marks
//! as Rune spans, chips, strikes and links (`docs/design.md` §9, Inline).
//!
//! A hidden spoiler keeps every mark that moves text (weight, italics, mono,
//! sizes, pads) and drops every one that paints, under a solid chip, so
//! revealing it changes no line and the row keeps its height.

use std::cell::RefCell;
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;

use rune::{Color, El, InlineImage, Link, LinkPaint, Span, TextStyle, rich};

use super::copy::spoken;
use super::doc::{Mark, PAD, Pad, Run};
use super::{Draw, Press};
use crate::design::{
    CHIP_PAD, CHIP_RADIUS, CODE_CHIP_OUTLINE_PT, LINK_UNDERLINE_ALPHA, Look, SPOILER_PAD_PT,
    TYPE_CODE, WEIGHT_MEDIUM, WEIGHT_SEMIBOLD,
};
use crate::emoji_images::{EmojiKey, EmojiPlace, inline_image};
use crate::strings;
use crate::timeline::TimelineHost;

thread_local! {
    /// Pad sizes already solved, by font, padding and font epoch.
    static PAD_SIZES: RefCell<HashMap<u64, f32>> = RefCell::new(HashMap::new());
}

/// The element key of text `index` of a message.
pub(crate) fn text_key(index: usize) -> String {
    format!("t{index}")
}

/// `run` set in `font` and `ink`, with its marks, selectable, its links
/// and spoilers pressable. `trailer` is drawn after it, uncopied: the
/// "(edited)" marker.
pub(super) fn text<S: TimelineHost>(
    run: &Run,
    font: TextStyle,
    ink: Color,
    draw: &Draw<'_>,
    trailer: Option<&Trailer>,
) -> El {
    let marks = marks(run, font, draw);
    let mut content = run.text.clone();
    let mut spans = marks.spans;
    let mut links = marks.links;
    let mut uncopied = marks.uncopied;
    if let Some(trailer) = trailer {
        let start = content.len();
        content.push(' ');
        content.push_str(&trailer.words);
        let range = start..content.len();
        spans.push(
            Span::new(range.start, range.end)
                .size(trailer.font.size)
                .color(trailer.ink),
        );
        if let Some(help) = &trailer.help {
            links.push(
                Link::new(range.start + 1..range.end, Press::Edited.id())
                    .role("note")
                    .label(help.clone())
                    .help(help.clone()),
            );
        }
        uncopied.push(range);
    }
    let (key, message) = (draw.key, draw.message);
    let mut el = rich(content, spans)
        .key(text_key(run.index))
        .font(font)
        .color(ink)
        .wrap_anywhere()
        .w_full()
        .span_radius(draw.look.radius(CHIP_RADIUS))
        .span_rings(marks.rings)
        .strikes(marks.strikes)
        .selectable(true)
        .selection_color(draw.look.palette.text_selection);
    for range in uncopied {
        el = el.uncopied(range);
    }
    for (range, image) in emoji_images(run, font, draw) {
        el = el.inline_image(range, image);
    }
    if let Some(spoken) = spoken(run) {
        let spoken = match trailer {
            Some(trailer) => format!("{spoken} {}", trailer.words),
            None => spoken,
        };
        el = el.label(spoken.clone()).a11y_value(spoken);
    }
    if !links.is_empty() {
        for link in links {
            el = el.link(link);
        }
        el = el
            .on_link(move |host: &mut S, id: &str, tx| {
                if let Some(press) = Press::parse(id) {
                    host.content_pressed(key, message, press, tx);
                }
            })
            // A link's own menu; anything else in the text has the message's.
            .on_link_secondary(move |host: &mut S, id: &str, at, tx| {
                let at = [at.x, at.y];
                match Press::parse(id) {
                    Some(Press::Url(url)) => host.link_menu_requested(key, url, at, tx),
                    _ => host.message_menu_requested(key, message, Some(at), tx),
                }
            });
    }
    el
}

/// Words drawn after a text, in their own size and colour, with a hint.
pub(super) struct Trailer {
    pub(super) words: String,
    pub(super) font: TextStyle,
    pub(super) ink: Color,
    /// What hovering or focusing it explains; none takes neither.
    pub(super) help: Option<String>,
}

/// A run's marks as Rune draws them.
#[derive(Default)]
struct Drawn {
    spans: Vec<Span>,
    rings: Vec<(usize, usize, Color, f32)>,
    strikes: Vec<(usize, usize, Option<Color>)>,
    links: Vec<Link>,
    uncopied: Vec<Range<usize>>,
}

/// The spans that change how `run` lays out in `font`, for measuring it.
pub(super) fn layout_spans(run: &Run, font: TextStyle, draw: &Draw<'_>) -> Vec<Span> {
    marks(run, font, draw).spans
}

fn marks(run: &Run, font: TextStyle, draw: &Draw<'_>) -> Drawn {
    let look = draw.look;
    let mut drawn = Drawn::default();
    // Inline code keeps its own weight in bold text, and a hidden spoiler
    // covers everything inside it, so both go after the rest: of
    // overlapping spans, the later one wins.
    let mut code = Vec::new();
    let mut covers = Vec::new();
    for mark in &run.marks {
        match mark {
            Mark::Bold(range) => {
                drawn
                    .spans
                    .push(Span::new(range.start, range.end).weight(WEIGHT_SEMIBOLD));
            }
            Mark::Italic(range) => drawn.spans.push(Span::new(range.start, range.end).italic()),
            Mark::Strike(range) if !run.is_hidden(range) => {
                drawn.strikes.push((range.start, range.end, None));
            }
            Mark::Code(range) => {
                code.push(code_chip(range, run.is_hidden(range), look, &mut drawn));
            }
            Mark::Pad { range, pad_pt } => {
                let pad_pt = match pad_pt {
                    Pad::Chip => look.space(CHIP_PAD),
                    Pad::Spoiler => SPOILER_PAD_PT,
                };
                drawn
                    .spans
                    .push(Span::new(range.start, range.end).size(pad_size(font, pad_pt)));
                drawn.uncopied.push(range.clone());
            }
            Mark::Link { range, url } if !run.is_hidden(range) => {
                drawn.spans.push(
                    Span::new(range.start, range.end)
                        .color(look.palette.link)
                        .underline(look.palette.link.alpha(LINK_UNDERLINE_ALPHA)),
                );
                let whole = LinkPaint::new().underline(look.palette.link);
                drawn.links.push(
                    Link::new(range.clone(), Press::Url(url.clone()).id())
                        .hover(whole)
                        .focused(whole)
                        .help(url.clone()),
                );
            }
            // Inside a hidden spoiler nothing shows through; an emoji's image is set over its
            // `:name:` by `emoji_images`.
            Mark::Strike(_) | Mark::Link { .. } | Mark::Emoji { .. } => {}
            Mark::Mention {
                range,
                known,
                bold,
                person,
            } => {
                let hidden = run.is_hidden(range);
                let span = mention_chip(range, *known, *bold, hidden, look);
                drawn.spans.push(span);
                // A known person's mention opens their card beside it.
                if let (Some(person), false) = (person, hidden) {
                    let press = Press::Person {
                        user: *person,
                        standalone: draw.standalone,
                    };
                    drawn.links.push(Link::new(range.clone(), press.id()));
                }
            }
            Mark::Spoiler {
                range,
                index,
                hidden: true,
            } => {
                covers.push(
                    Span::new(range.start, range.end)
                        .color(Color::TRANSPARENT)
                        .bg(look.palette.line_strong, SPOILER_PAD_PT),
                );
                // One inside another hidden one takes no press until the
                // outer is revealed: it cannot be seen.
                let enclosed = run.hidden().any(|outer| {
                    outer != *range && outer.start <= range.start && range.end <= outer.end
                });
                if !enclosed {
                    drawn.links.push(spoiler_press(run, range, *index, look));
                }
            }
            Mark::Spoiler { range, .. } => {
                drawn.spans.push(
                    Span::new(range.start, range.end)
                        .bg(look.palette.surface_hover, SPOILER_PAD_PT),
                );
            }
        }
    }
    drawn.spans.extend(code);
    drawn.spans.extend(covers);
    drawn
}

/// Each custom emoji of `run` drawn in place of its `:name:` (§9, Emoji); a hidden spoiler's
/// keep their room, clear, so revealing moves no line.
fn emoji_images(run: &Run, font: TextStyle, draw: &Draw<'_>) -> Vec<(Range<usize>, InlineImage)> {
    run.marks
        .iter()
        .filter_map(|mark| {
            let Mark::Emoji { range, id } = mark else {
                return None;
            };
            let key = EmojiKey {
                account: draw.key.account,
                id: *id,
            };
            let place = EmojiPlace {
                font,
                jumbo: run.jumbo,
                hidden: run.is_hidden(range),
            };
            let image = inline_image(&draw.emoji.look(key), place, draw.look)?;
            Some((range.clone(), image))
        })
        .collect()
}

/// Inline code: mono at `type.small`, on its outlined chip unless hidden.
fn code_chip(range: &Range<usize>, hidden: bool, look: &Look, drawn: &mut Drawn) -> Span {
    let span = Span::new(range.start, range.end)
        .mono()
        .size(look.font(TYPE_CODE).size)
        .weight(look.font(TYPE_CODE).weight);
    if hidden {
        return span;
    }
    drawn.rings.push((
        range.start,
        range.end,
        look.palette.line_subtle,
        CODE_CHIP_OUTLINE_PT,
    ));
    span.color(look.palette.text_primary)
        .bg(look.palette.surface_code, look.space(CHIP_PAD))
}

/// A mention: weight 500 (600 in bold), on its chip unless hidden; a
/// person the server does not know is muted.
fn mention_chip(range: &Range<usize>, known: bool, bold: bool, hidden: bool, look: &Look) -> Span {
    let weight = if bold { WEIGHT_SEMIBOLD } else { WEIGHT_MEDIUM };
    let span = Span::new(range.start, range.end).weight(weight);
    match (hidden, known) {
        (true, _) => span,
        (false, true) => span
            .color(look.palette.link)
            .bg(look.palette.accent_soft, look.space(CHIP_PAD)),
        (false, false) => span
            .color(look.palette.text_muted)
            .bg(look.palette.surface_hover, look.space(CHIP_PAD)),
    }
}

/// What reveals a hidden spoiler: a press on its chip, pads included.
fn spoiler_press(run: &Run, range: &Range<usize>, index: u32, look: &Look) -> Link {
    let start = range.start.saturating_sub(PAD.len());
    let end = (range.end + PAD.len()).min(run.text.len());
    Link::new(start..end, Press::Spoiler(index).id())
        .role("button")
        .label(strings::spoiler_hidden())
        .hover(LinkPaint::new().bg(look.palette.text_muted))
}

/// The size that sets a [`PAD`] in `font` to `pad_pt` points, so the words
/// beside a chip sit where a page's `padding: 0 4px` puts them. Solved
/// twice: a variable font's spaces change width with its size.
fn pad_size(font: TextStyle, pad_pt: f32) -> f32 {
    let mut hasher = DefaultHasher::new();
    (font, pad_pt.to_bits(), rune::text::font_epoch()).hash(&mut hasher);
    let key = hasher.finish();
    if let Some(size) = PAD_SIZES.with_borrow(|sizes| sizes.get(&key).copied()) {
        return size;
    }
    let probe = PAD.repeat(64);
    let width = |size: f32| {
        let style = TextStyle { size, ..font };
        (rune::text::measure(&probe, &[], style, None).0 / 64.0).max(0.1)
    };
    let first = font.size * pad_pt / width(font.size);
    let size = first * pad_pt / width(first);
    PAD_SIZES.with_borrow_mut(|sizes| sizes.insert(key, size));
    size
}
