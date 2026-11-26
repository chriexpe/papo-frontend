//! GIPHY's grid (`docs/design.md` §16.3): three columns, each tile as tall as its GIF's width
//! and height give at the column's width (reserved before its picture loads, so nothing moves
//! when it does), each in the shortest column, in GIPHY's order.

use papo_sdk::Gif;

/// The grid's columns.
pub(crate) const COLUMNS: usize = 3;
/// The space between tiles, across and down, in points.
pub(crate) const GAP_PT: f32 = 4.0;
/// A tile's tallest and shortest, against its width: a GIF of odd proportions still fits.
const MAX_RATIO: f32 = 3.0;
const MIN_RATIO: f32 = 0.3;

/// One tile in a column.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placed {
    /// The GIF's place in the shelf.
    pub(crate) at: usize,
    pub(crate) height_pt: f32,
}

/// `gifs` in [`COLUMNS`] columns `width_pt` wide.
pub(crate) fn columns(gifs: &[Gif], width_pt: f32) -> [Vec<Placed>; COLUMNS] {
    let mut columns: [Vec<Placed>; COLUMNS] = Default::default();
    let mut heights = [0.0_f32; COLUMNS];
    for (at, gif) in gifs.iter().enumerate() {
        let height_pt = height(gif, width_pt);
        // The shortest; on a tie, the leftmost.
        let column = (0..COLUMNS)
            .min_by(|a, b| heights[*a].total_cmp(&heights[*b]))
            .unwrap_or(0);
        columns[column].push(Placed { at, height_pt });
        heights[column] += height_pt + GAP_PT;
    }
    columns
}

/// The width of a column in a grid `inner_pt` wide.
pub(crate) fn column_width(inner_pt: f32) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "three columns")]
    let columns = COLUMNS as f32;
    ((inner_pt - (columns - 1.0) * GAP_PT) / columns).max(1.0)
}

/// A tile's height at `width_pt`: from the preview's size, else the original's, else square.
fn height(gif: &Gif, width_pt: f32) -> f32 {
    let sizes = [
        (gif.preview_width, gif.preview_height),
        (gif.width, gif.height),
    ];
    let ratio = sizes
        .into_iter()
        .find(|(width, height)| *width > 0 && *height > 0)
        .map_or(1.0, |(width, height)| {
            #[expect(clippy::cast_precision_loss, reason = "pixel sizes in the thousands")]
            let ratio = height as f32 / width as f32;
            ratio
        });
    width_pt * ratio.clamp(MIN_RATIO, MAX_RATIO)
}

#[cfg(test)]
mod tests {
    use papo_sdk::Gif;

    use super::{COLUMNS, columns};

    fn gif(width: u32, height: u32) -> Gif {
        Gif {
            preview_width: width,
            preview_height: height,
            ..Gif::default()
        }
    }

    #[test]
    fn each_tile_goes_in_the_shortest_column_in_giphys_order() {
        // 138 wide: a square, a tall one, a wide one, then the shortest column is the third.
        let gifs = [gif(200, 200), gif(200, 400), gif(200, 100), gif(200, 200)];
        let placed = columns(&gifs, 138.0);
        let order: Vec<Vec<usize>> = placed
            .iter()
            .map(|column| column.iter().map(|tile| tile.at).collect())
            .collect();
        assert_eq!(order, [vec![0], vec![1], vec![2, 3]]);
        assert_eq!(
            placed[1][0].height_pt, 276.0,
            "as tall as its proportions give"
        );
        assert_eq!(placed.len(), COLUMNS);
    }

    #[test]
    fn a_gif_without_sizes_is_square_and_odd_proportions_are_bounded() {
        let placed = columns(&[gif(0, 0), gif(10, 1000)], 100.0);
        assert_eq!(placed[0][0].height_pt, 100.0);
        assert_eq!(placed[1][0].height_pt, 300.0);
    }
}
