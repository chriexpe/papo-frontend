//! How an album of two or more pictures is laid out (`docs/design.md` §14.4): its shape, how
//! many cells it draws, the "+N" count, and each cell's place in its box. Drawing and cropping
//! the pictures into the cells are the client's.

use super::{MediaBounds, MediaBox, MediaFit, largest_of_shape};

/// The most cells an album draws; the rest are counted on the last one ("+N").
pub const ALBUM_CELLS: usize = 4;

/// The gap between an album's cells, in points.
pub const ALBUM_GAP_PT: f32 = 2.0;

/// How an album of two or more pictures is laid out (§14.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlbumLayout {
    /// Two side by side, 3 : 2.
    Two,
    /// One tall on the left (two thirds of the width), two stacked on the right; 4 : 3.
    Three,
    /// Two by two, 4 : 3, with `more` pictures counted on the fourth.
    Four {
        /// How many pictures follow the fourth ("+N"); zero for exactly four.
        more: usize,
    },
}

impl AlbumLayout {
    /// The layout of `count` pictures; none for fewer than two.
    #[must_use]
    pub fn for_count(count: usize) -> Option<AlbumLayout> {
        match count {
            0 | 1 => None,
            2 => Some(AlbumLayout::Two),
            3 => Some(AlbumLayout::Three),
            _ => Some(AlbumLayout::Four {
                more: count - ALBUM_CELLS,
            }),
        }
    }

    /// Its shape, width over height.
    #[must_use]
    pub fn aspect(self) -> f32 {
        match self {
            AlbumLayout::Two => 3.0 / 2.0,
            AlbumLayout::Three | AlbumLayout::Four { .. } => 4.0 / 3.0,
        }
    }

    /// How many pictures it draws.
    #[must_use]
    pub fn shown(self) -> usize {
        match self {
            AlbumLayout::Two => 2,
            AlbumLayout::Three => 3,
            AlbumLayout::Four { .. } => ALBUM_CELLS,
        }
    }

    /// The pictures counted on the last cell ("+N").
    #[must_use]
    pub fn more(self) -> usize {
        match self {
            AlbumLayout::Four { more } => more,
            AlbumLayout::Two | AlbumLayout::Three => 0,
        }
    }

    /// The album's box within `bounds`: as wide as they allow at its shape. Its cells are
    /// cropped to fill.
    #[must_use]
    pub fn album_box(self, bounds: MediaBounds) -> MediaBox {
        let (width_pt, height_pt) = largest_of_shape(self.aspect(), bounds.sane());
        MediaBox {
            width_pt,
            height_pt,
            fit: MediaFit::Fill,
        }
    }

    /// Each drawn picture's cell within `album`, in order, [`ALBUM_GAP_PT`] apart: left to
    /// right, then top to bottom (for three, the tall one first).
    #[must_use]
    pub fn cells(self, album: MediaBox) -> Vec<AlbumCell> {
        let (width, height, gap) = (album.width_pt, album.height_pt, ALBUM_GAP_PT);
        // Cells may fall on half points; one too small for the gap is empty, never negative.
        let half_width = ((width - gap) / 2.0).max(0.0);
        let half_height = ((height - gap) / 2.0).max(0.0);
        let cell = |x_pt, y_pt, width_pt, height_pt| AlbumCell {
            x_pt,
            y_pt,
            width_pt,
            height_pt,
        };
        match self {
            AlbumLayout::Two => vec![
                cell(0.0, 0.0, half_width, height),
                cell(half_width + gap, 0.0, half_width, height),
            ],
            AlbumLayout::Three => {
                let tall = ((width - gap) * 2.0 / 3.0).max(0.0);
                let right = (width - gap - tall).max(0.0);
                vec![
                    cell(0.0, 0.0, tall, height),
                    cell(tall + gap, 0.0, right, half_height),
                    cell(tall + gap, half_height + gap, right, half_height),
                ]
            }
            AlbumLayout::Four { .. } => vec![
                cell(0.0, 0.0, half_width, half_height),
                cell(half_width + gap, 0.0, half_width, half_height),
                cell(0.0, half_height + gap, half_width, half_height),
                cell(half_width + gap, half_height + gap, half_width, half_height),
            ],
        }
    }
}

/// One album cell, from the album's top left, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AlbumCell {
    /// Left edge.
    pub x_pt: f32,
    /// Top edge.
    pub y_pt: f32,
    /// Width.
    pub width_pt: f32,
    /// Height.
    pub height_pt: f32,
}
