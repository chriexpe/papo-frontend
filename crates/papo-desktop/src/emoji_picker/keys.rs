//! The Emoji tab's cursor (`docs/design.md` §16.5): into the results from the search, by cell
//! and by row, to a row's ends, and up from the first row back to the search; and the element
//! keys that name the cell it is on and bring its row into view.

use super::{EmojiPicker, Row, Section, block_key, block_rows_key, cell_row_key};

/// A key the results take.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    /// Into the results (Tab), or on the cell.
    Enter,
}

impl EmojiPicker {
    /// The keyboard moves. Returns the cell it is on now; none is the search.
    pub(crate) fn step(&mut self, step: Step) -> Option<usize> {
        let last = self.cells.len().checked_sub(1)?;
        let Some(at) = self.cursor else {
            if step == Step::Down || step == Step::Enter {
                self.cursor = Some(0);
            }
            return self.cursor;
        };
        let (row, column) = self.place(at)?;
        self.cursor = match step {
            Step::Left => Some(at.saturating_sub(1)),
            Step::Right => Some((at + 1).min(last)),
            // From the first row, none: back to the search.
            Step::Up => self
                .cells_row_from(row, -1)
                .map(|cells| cells[column.min(cells.len() - 1)]),
            Step::Down => self
                .cells_row_from(row, 1)
                .map_or(Some(at), |cells| Some(cells[column.min(cells.len() - 1)])),
            Step::Home => self.row_cells(row).first().copied(),
            Step::End => self.row_cells(row).last().copied(),
            Step::Enter => Some(at),
        };
        self.cursor
    }

    /// The element key of the cell the keyboard is on: the search names it as its active
    /// descendant.
    pub(crate) fn chosen_key(&self) -> Option<String> {
        let (_, section, _) = self.chosen_place()?;
        Some(self.chosen()?.key(section))
    }

    /// The key path of the row of cells the keyboard is on, through the section that holds it,
    /// so a reveal reaches it while neither is built.
    pub(crate) fn chosen_row_path(&self) -> Option<String> {
        let (_, section, row) = self.chosen_place()?;
        Some(format!(
            "{}/{}/{}",
            block_key(section),
            block_rows_key(section),
            cell_row_key(section, row)
        ))
    }

    /// Where the keyboard's cell is: its row in the list, its section, and its row in the
    /// section's cells.
    fn chosen_place(&self) -> Option<(usize, Section, usize)> {
        let (row, _) = self.place(self.cursor?)?;
        let block = self.blocks.iter().find(|block| block.rows.contains(&row))?;
        Some((row, block.section, row - block.cell_rows().start))
    }

    /// The row and column of cell `at`.
    fn place(&self, at: usize) -> Option<(usize, usize)> {
        self.rows
            .iter()
            .enumerate()
            .find_map(|(row, line)| match line {
                Row::Cells { cells, .. } => cells
                    .iter()
                    .position(|cell| *cell == at)
                    .map(|column| (row, column)),
                Row::Heading { .. } => None,
            })
    }

    /// The nearest row of cells from `row` going `step` (±1), skipping headings.
    fn cells_row_from(&self, row: usize, step: isize) -> Option<&[usize]> {
        let mut at = row;
        loop {
            at = at.checked_add_signed(step)?;
            match self.rows.get(at)? {
                Row::Cells { cells, .. } => return Some(cells),
                Row::Heading { .. } => {}
            }
        }
    }

    fn row_cells(&self, row: usize) -> &[usize] {
        match self.rows.get(row) {
            Some(Row::Cells { cells, .. }) => cells,
            _ => &[],
        }
    }
}
