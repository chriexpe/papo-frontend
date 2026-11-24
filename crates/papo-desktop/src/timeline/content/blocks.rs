//! Blocks drawn (`docs/design.md` §9, Blocks): paragraphs, headings, quotes,
//! lists and tasks, rules and blank lines. Code and tables are their own
//! modules. Blocks are 4 points apart and add no margin of their own.

use rune::text::measure;
use rune::{Align, Color, El, col, el, icon, row, text};

use super::doc::{Item, Marker, Node};
use super::inline::{Trailer, text as run_text};
use super::{Draw, code, table};
use crate::design::{
    Font, LIST_ITEM_GAP_PT, LIST_MARKER, LIST_NUMBER_GAP, Look, MESSAGE_BLOCK_GAP, QUOTE_BAR_PT,
    QUOTE_INDENT, RULE_PT, TASK_BOX, TASK_BOX_OUTLINE_PT, TASK_BOX_RADIUS, TASK_CHECK, TYPE_BODY,
    TYPE_JUMBO, type_heading,
};
use crate::strings;
use crate::timeline::TimelineHost;

/// A list's numbers: body text with figures all one width.
const TYPE_NUMBER: Font = Font(TYPE_BODY.0.features("'tnum' 1"));

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
    let look = draw.look;
    match node {
        Node::Paragraph(run) => run_text::<S>(run, look.font(TYPE_BODY), ink, draw, trailer),
        Node::Heading { level, run } => {
            let ink = if *level == 6 {
                look.palette.text_secondary
            } else {
                ink
            };
            run_text::<S>(run, look.font(type_heading(*level)), ink, draw, trailer)
                .role("heading")
                .level(u32::from(*level))
        }
        Node::Quote(inner) => col()
            .w_full()
            .gap(look.space(MESSAGE_BLOCK_GAP))
            .pl(QUOTE_BAR_PT + look.space(QUOTE_INDENT))
            .border_sides([0.0, 0.0, 0.0, QUOTE_BAR_PT], look.palette.line_strong)
            .role("blockquote")
            .children(nodes::<S>(inner, draw, look.palette.text_secondary, None)),
        Node::List(items) => {
            let column_pt = marker_column_pt(items, look);
            col().w_full().gap(LIST_ITEM_GAP_PT).role("list").children(
                items.iter().enumerate().map(|(index, item)| {
                    item_el::<S>(item, column_pt, draw, ink).key(format!("i{index}"))
                }),
            )
        }
        Node::Code { language, run } => code::block::<S>(language.as_deref(), run, draw),
        Node::Rule => el()
            .w_full()
            .h(RULE_PT)
            .fill(look.palette.line_subtle)
            .role("separator"),
        Node::Table {
            alignments,
            header,
            rows,
        } => table::table::<S>(alignments, header, rows, draw, ink),
        Node::Blank => el().w_full().h(look.font(TYPE_BODY).line_height),
        Node::Jumbo(run) => run_text::<S>(run, look.font(TYPE_JUMBO), ink, draw, trailer),
    }
}

/// The marker column's width for `items`: 1.4 em, or as wide as the widest
/// number and its gap, so "9." and "10." share their dot.
fn marker_column_pt(items: &[Item], look: &Look) -> f32 {
    let Some(Marker::Number { width, .. }) = items.first().map(|item| item.marker) else {
        return look.text_multiple(LIST_MARKER);
    };
    let widest = format!("{}.", "0".repeat(width));
    let number_pt = measure(&widest, &[], look.font(TYPE_NUMBER), None).0;
    (number_pt + look.text_multiple(LIST_NUMBER_GAP)).max(look.text_multiple(LIST_MARKER))
}

/// A list item: its marker in a column `column_pt` wide, then its blocks.
fn item_el<S: TimelineHost>(item: &Item, column_pt: f32, draw: &Draw<'_>, ink: Color) -> El {
    let look = draw.look;
    let ink = match item.marker {
        Marker::Task { done: true } => look.palette.text_secondary,
        _ => ink,
    };
    row()
        .w_full()
        .items_start()
        .role("listitem")
        .child(marker_el(item.marker, column_pt, draw))
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
fn marker_el(marker: Marker, column_pt: f32, draw: &Draw<'_>) -> El {
    let look = draw.look;
    let line = look.font(TYPE_BODY).line_height;
    let column = row().key("marker").w(column_pt).h(line).center_y();
    match marker {
        Marker::Bullet(_) => column.justify_center().child(
            text(marker.text())
                .font(look.font(TYPE_BODY))
                .color(look.palette.text_secondary)
                .a11y_hidden(),
        ),
        Marker::Number { .. } => column
            .justify_end()
            .pr(look.text_multiple(LIST_NUMBER_GAP))
            .child(
                text(marker.text())
                    .font(look.font(TYPE_NUMBER))
                    .color(look.palette.text_secondary)
                    .align(Align::End),
            ),
        Marker::Task { done } => {
            let square = el()
                .size(look.text_multiple(TASK_BOX), look.text_multiple(TASK_BOX))
                .radius(look.radius(TASK_BOX_RADIUS))
                .center()
                .role("checkbox")
                .checked(done)
                .label(if done {
                    strings::task_done()
                } else {
                    strings::task_to_do()
                });
            let square = if done {
                square.fill(look.palette.accent).child(
                    icon(&crate::icons::CHECK, look.text_multiple(TASK_CHECK))
                        .color(look.palette.accent_on),
                )
            } else {
                square.border(TASK_BOX_OUTLINE_PT, look.palette.line_strong)
            };
            column.child(square)
        }
    }
}
