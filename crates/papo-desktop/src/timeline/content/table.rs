//! A table (`docs/design.md` §9, Blocks): text at 0.93×, a header row on
//! `surface.sidebar` at weight 600, 1-point lines between cells, the
//! delimiter row's alignment. Each column is as wide as its widest cell, up
//! to 28 characters, past which cells wrap; a table wider than the message
//! scrolls sideways inside it and never squeezes its columns.

use papo_sdk_ui::message_content::TableAlignment;
use rune::{Align, Color, El, TextStyle, col, el, row};

use super::Draw;
use super::doc::Run;
use super::inline::{layout_spans, text as run_text};
use crate::design::{
    TABLE_CELL_CHARS, TABLE_CELL_PAD_PT, TABLE_LINE_PT, TYPE_TABLE, WEIGHT_SEMIBOLD,
};
use crate::timeline::TimelineHost;

/// The table of `header` and `rows`, its text in `ink`.
pub(super) fn table<S: TimelineHost>(
    alignments: &[TableAlignment],
    header: &[Run],
    rows: &[Vec<Run>],
    draw: &Draw<'_>,
    ink: Color,
) -> El {
    let look = draw.look;
    let header_font = TextStyle {
        weight: WEIGHT_SEMIBOLD,
        ..look.font(TYPE_TABLE)
    };
    let widths = column_widths(header, rows, header_font, draw);
    let line = |row: usize, column: usize| {
        let top = if row == 0 { TABLE_LINE_PT } else { 0.0 };
        let left = if column == 0 { TABLE_LINE_PT } else { 0.0 };
        [top, TABLE_LINE_PT, TABLE_LINE_PT, left]
    };
    let cell = |run: &Run, row: usize, column: usize, font: TextStyle| {
        let align = match alignments.get(column) {
            Some(TableAlignment::Center) => Align::Center,
            Some(TableAlignment::Right) => Align::End,
            _ => Align::Start,
        };
        let (vertical, horizontal) = TABLE_CELL_PAD_PT;
        el().key(format!("c{column}"))
            .w(widths.get(column).copied().unwrap_or_default())
            .py(vertical)
            .px(horizontal)
            .border_sides(line(row, column), look.palette.line_subtle)
            .border_layout()
            .child(run_text::<S>(run, font, ink, draw, None).align(align))
    };
    // The cells carry the header's fill: the row is as wide as the
    // scroll area, past the last column.
    let header_row = row()
        .key("r0")
        .role("row")
        .children(header.iter().enumerate().map(|(column, run)| {
            cell(run, 0, column, header_font)
                .fill(look.palette.surface_sidebar)
                .role("columnheader")
        }));
    let body = rows.iter().enumerate().map(|(index, cells)| {
        row().key(format!("r{}", index + 1)).role("row").children(
            cells.iter().enumerate().map(|(column, run)| {
                cell(run, index + 1, column, look.font(TYPE_TABLE)).role("cell")
            }),
        )
    });
    el().key("table")
        .w_full()
        .scroll_x()
        .child(col().role("table").child(header_row).children(body))
}

/// Each column's width in points, lines and padding included: its widest
/// cell on one line, up to [`TABLE_CELL_CHARS`] characters.
fn column_widths(
    header: &[Run],
    rows: &[Vec<Run>],
    header_font: TextStyle,
    draw: &Draw<'_>,
) -> Vec<f32> {
    let look = draw.look;
    let most = rune::text::measure(
        &"0".repeat(TABLE_CELL_CHARS),
        &[],
        look.font(TYPE_TABLE),
        None,
    )
    .0;
    let natural = |run: &Run, font: TextStyle| {
        let spans = layout_spans(run, font, draw);
        rune::text::measure(&run.text, &spans, font, None)
            .0
            .min(most)
    };
    let columns = header
        .len()
        .max(rows.iter().map(Vec::len).max().unwrap_or(0));
    let (_, horizontal) = TABLE_CELL_PAD_PT;
    (0..columns)
        .map(|column| {
            let widest = header
                .get(column)
                .map(|run| natural(run, header_font))
                .into_iter()
                .chain(
                    rows.iter()
                        .filter_map(|cells| cells.get(column))
                        .map(|run| natural(run, look.font(TYPE_TABLE))),
                )
                .fold(0.0, f32::max);
            widest.ceil() + 2.0 * horizontal + 2.0 * TABLE_LINE_PT
        })
        .collect()
}
