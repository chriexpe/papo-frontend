//! Blocks drawn (`docs/design.md` §9, Blocks): paragraphs, headings, quotes,
//! lists and tasks, rules and blank lines. Code and tables are their own
//! modules. Blocks are 4 points apart and add no margin of their own.

use rune::text::measure;
use rune::{Align, Color, El, col, el, icon, row, text};

use super::doc::{Item, Marker, Node};
use super::inline::{Trailer, text as run_text};
use super::{Draw, code, table};
use crate::design::{
    LIST_ITEM_GAP_PT, LIST_MARKER_PT, LIST_NUMBER_GAP_PT, MESSAGE_BLOCK_GAP_PT, QUOTE_BAR_PT,
    QUOTE_INDENT_PT, RULE_PT, TASK_BOX_OUTLINE_PT, TASK_BOX_PT, TASK_BOX_RADIUS_PT, TASK_CHECK_PT,
    TYPE_BODY, TYPE_JUMBO, type_heading,
};
use crate::strings;
use crate::timeline::TimelineHost;

/// `nodes`, their text in `ink`. `trailer` follows the last one's last
/// line when that is a paragraph or a heading. A block that is one text
/// keeps the text's key (`t{index}`, which a copy finds it by); the others
/// are keyed `b{index}`.
pub(super) fn nodes<S: TimelineHost>(
    nodes: &[Node],
    draw: &Draw<'_>,
    ink: Color,
    trailer: Option<&Trailer>,
) -> Vec<El> {
    let last = nodes.len().saturating_sub(1);
    nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let trailer = trailer.filter(|_| index == last);
            let el = node_el::<S>(node, draw, ink, trailer);
            match node {
                Node::Paragraph(_) | Node::Heading { .. } | Node::Jumbo(_) => el,
                _ => el.key(format!("b{index}")),
            }
        })
        .collect()
}

fn node_el<S: TimelineHost>(
    node: &Node,
    draw: &Draw<'_>,
    ink: Color,
    trailer: Option<&Trailer>,
) -> El {
    let palette = draw.palette;
    match node {
        Node::Paragraph(run) => run_text::<S>(run, TYPE_BODY, ink, draw, trailer),
        Node::Heading { level, run } => {
            let ink = if *level == 6 {
                palette.text_secondary
            } else {
                ink
            };
            run_text::<S>(run, type_heading(*level), ink, draw, trailer)
                .role("heading")
                .level(u32::from(*level))
        }
        Node::Quote(inner) => col()
            .w_full()
            .gap(MESSAGE_BLOCK_GAP_PT)
            .pl(QUOTE_BAR_PT + QUOTE_INDENT_PT)
            .border_sides([0.0, 0.0, 0.0, QUOTE_BAR_PT], palette.line_strong)
            .role("blockquote")
            .children(nodes::<S>(inner, draw, palette.text_secondary, None)),
        Node::List(items) => col().w_full().gap(LIST_ITEM_GAP_PT).role("list").children(
            items
                .iter()
                .enumerate()
                .map(|(index, item)| item_el::<S>(item, draw, ink).key(format!("i{index}"))),
        ),
        Node::Code { language, run } => code::block::<S>(language.as_deref(), run, draw),
        Node::Rule => el()
            .w_full()
            .h(RULE_PT)
            .fill(palette.line_subtle)
            .role("separator"),
        Node::Table {
            alignments,
            header,
            rows,
        } => table::table::<S>(alignments, header, rows, draw, ink),
        Node::Blank => el().w_full().h(TYPE_BODY.line_height),
        Node::Jumbo(run) => run_text::<S>(run, TYPE_JUMBO, ink, draw, trailer),
    }
}

/// A list item: its marker in a column of its own, then its blocks.
fn item_el<S: TimelineHost>(item: &Item, draw: &Draw<'_>, ink: Color) -> El {
    let palette = draw.palette;
    let ink = match item.marker {
        Marker::Task { done: true } => palette.text_secondary,
        _ => ink,
    };
    row()
        .w_full()
        .items_start()
        .role("listitem")
        .child(marker_el(item.marker, draw))
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(LIST_ITEM_GAP_PT)
                .children(nodes::<S>(&item.nodes, draw, ink, None)),
        )
}

/// The marker column: a bullet centred, a number right-aligned so the
/// numbers share their dot, or a task's box, on the item's first line.
fn marker_el(marker: Marker, draw: &Draw<'_>) -> El {
    let palette = draw.palette;
    let line = TYPE_BODY.line_height;
    let column = row().key("marker").h(line).center_y();
    match marker {
        Marker::Bullet(_) => column.w(LIST_MARKER_PT).justify_center().child(
            text(marker.text())
                .font(TYPE_BODY)
                .color(palette.text_secondary)
                .a11y_hidden(),
        ),
        Marker::Number { width, .. } => {
            let widest = format!("{}.", "0".repeat(width));
            let number_pt = measure(&widest, &[], TYPE_BODY, None).0;
            column
                .w((number_pt + LIST_NUMBER_GAP_PT).max(LIST_MARKER_PT))
                .justify_end()
                .pr(LIST_NUMBER_GAP_PT)
                .child(
                    text(marker.text())
                        .font(TYPE_BODY.features("'tnum' 1"))
                        .color(palette.text_secondary)
                        .align(Align::End),
                )
        }
        Marker::Task { done } => {
            let square = el()
                .size(TASK_BOX_PT, TASK_BOX_PT)
                .radius(TASK_BOX_RADIUS_PT)
                .center()
                .role("checkbox")
                .checked(done)
                .label(if done {
                    strings::task_done()
                } else {
                    strings::task_to_do()
                });
            let square = if done {
                square
                    .fill(palette.accent)
                    .child(icon(&crate::icons::CHECK, TASK_CHECK_PT).color(palette.accent_on))
            } else {
                square.border(TASK_BOX_OUTLINE_PT, palette.line_strong)
            };
            column.w(LIST_MARKER_PT).child(square)
        }
    }
}
