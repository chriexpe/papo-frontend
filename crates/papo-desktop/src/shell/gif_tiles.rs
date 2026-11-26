//! The GIF tab's tiles from the shell's side (`docs/design.md` §16.3): where each is, which are
//! near enough to load (their pictures fetched from GIPHY through the SDK's anonymous source,
//! `MediaSource::External`, and kept in the media store with the conversation's), and which show
//! enough to animate: at least half within the tab's list, as the conversation's GIFs do. The
//! next page is asked for as the last tiles come near.
//!
//! The grid reports its place once per move; each tile's place follows from the masonry, so a
//! scroll costs one report, not one per tile.

use std::collections::BTreeSet;

use papo_sdk::{AccountId, Gif, MediaSource};
use rune::Tx;

use super::Shell;
use super::received_media::shown_gifs;
use crate::gif_picker::masonry::{self, GAP_PT};
use crate::gif_picker::view::{tile_media, tile_url};
use crate::media_store::{MediaKey, Want};

/// Tiles this far past the list's edges (in list heights) already load.
const NEAR_LISTS: f32 = 1.0;
/// The next page is asked for once one of the last this many tiles comes near.
const NEAR_END_TILES: usize = 6;
/// The only hosts tiles are fetched from: GIPHY's. A saved GIF naming another is not drawn.
const GIPHY_HOST: &str = "giphy.com";

/// Where the open tab's list and grid are.
#[derive(Default)]
pub(crate) struct TilePlaces {
    /// The list's viewport in the window.
    viewport: Option<[f32; 4]>,
    /// The grid's rect in the window: the tiles' places follow from it.
    grid: Option<[f32; 4]>,
    /// The tile under the pointer or the keyboard: it plays when GIFs don't animate on their own.
    hovered: Option<MediaKey>,
    /// The tiles near the list last time, by their place, for telling a change.
    near: BTreeSet<usize>,
}

impl TilePlaces {
    /// The tab closed: no list or grid is anywhere.
    pub(crate) fn closed(&mut self) {
        *self = TilePlaces::default();
    }

    /// The tab shows other tiles (another chip, a search). The list and the grid stay where
    /// they are (they report their places only when they move), so the new tiles' places follow
    /// from them at once.
    pub(crate) fn tiles_changed(&mut self) {
        self.near.clear();
        self.hovered = None;
    }

    /// Each of `gifs`' tiles, by its place in the shelf: its picture's key and its rect in the
    /// window, as the masonry lays them out in the grid.
    fn rects(&self, account: AccountId, gifs: &[Gif]) -> Vec<(usize, MediaKey, [f32; 4])> {
        let Some([x, y, width, _]) = self.grid else {
            return Vec::new();
        };
        let column_pt = masonry::column_width(width);
        let mut rects = Vec::with_capacity(gifs.len());
        for (column, placed) in masonry::columns(gifs, column_pt).iter().enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "three columns")]
            let left = x + column as f32 * (column_pt + GAP_PT);
            let mut top = y;
            for tile in placed {
                let key = tile_media(account, &gifs[tile.at]);
                rects.push((tile.at, key, [left, top, column_pt, tile.height_pt]));
                top += tile.height_pt + GAP_PT;
            }
        }
        rects
    }

    /// The places of the tiles within `NEAR_LISTS` list heights of the list.
    fn near_list(&self, rects: &[(usize, MediaKey, [f32; 4])]) -> BTreeSet<usize> {
        let Some([_, top, _, height]) = self.viewport else {
            return BTreeSet::new();
        };
        let (from, to) = (top - height * NEAR_LISTS, top + height * (1.0 + NEAR_LISTS));
        rects
            .iter()
            .filter(|(_, _, [_, y, _, tall])| y + tall >= from && *y <= to)
            .map(|(at, _, _)| *at)
            .collect()
    }

    /// The tiles that show enough to animate (each with how much shows), those GIFs that
    /// animate on their own when `animate`, else only the one under the pointer or keyboard.
    fn showing(
        &self,
        rects: &[(usize, MediaKey, [f32; 4])],
        animate: bool,
    ) -> Vec<(f32, MediaKey)> {
        let Some([_, top, _, height]) = self.viewport else {
            return Vec::new();
        };
        let hovered = self.hovered;
        shown_gifs(
            rects.iter().map(|(_, key, rect)| (*key, *rect)),
            (top, top + height),
            |key| animate || hovered == Some(key),
        )
    }
}

/// Whether `gif`'s tile may be fetched: from GIPHY over HTTPS only.
fn fetchable(gif: &Gif) -> bool {
    url::Url::parse(tile_url(gif)).is_ok_and(|url| {
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| host == GIPHY_HOST || host.ends_with(&format!(".{GIPHY_HOST}")))
    })
}

impl Shell {
    /// The open tab's tiles and their places, as the shown field's account sees them.
    fn gif_rects(&self) -> Vec<(usize, MediaKey, [f32; 4])> {
        let (Some(picker), Some((key, _))) = (self.gif_picker(), self.composer.shown()) else {
            return Vec::new();
        };
        self.gif.tiles.rects(key.account, picker.gifs())
    }

    /// The open tab's tiles near its list want their pictures: each at its tile's size, fetched
    /// from its address. Returns the wants and each one's source.
    pub(super) fn gif_tiles_wanted(&self) -> Vec<(Want, MediaSource)> {
        let Some(picker) = self.gif_picker() else {
            return Vec::new();
        };
        let scale = self.scale;
        self.gif_rects()
            .into_iter()
            .filter(|(at, _, _)| self.gif.tiles.near.contains(at))
            .filter_map(|(at, key, [_, _, width, height])| {
                let gif = picker.gifs().get(at).filter(|gif| fetchable(gif))?;
                let box_pt = width.max(height);
                let want = Want {
                    key,
                    thumbnail_px: None,
                    box_pt,
                    box_px: box_pt * scale,
                    gif: true,
                };
                Some((want, MediaSource::External(tile_url(gif).to_owned())))
            })
            .collect()
    }

    /// The GIF tab's tiles that show enough to animate.
    pub(super) fn gif_tiles_showing(&self, animate: bool) -> Vec<(f32, MediaKey)> {
        self.gif.tiles.showing(&self.gif_rects(), animate)
    }

    /// The grid is at `rect`: what loads, what animates and whether the next page is due may
    /// change.
    pub(crate) fn gif_grid_moved(&mut self, rect: [f32; 4], tx: &mut Tx) {
        self.gif.tiles.grid = Some(rect);
        self.gif_places_changed(tx);
    }

    /// The tab's list is at `rect`.
    pub(crate) fn gif_list_moved(&mut self, rect: [f32; 4], tx: &mut Tx) {
        self.gif.tiles.viewport = Some(rect);
        self.gif_places_changed(tx);
    }

    /// The pointer or the keyboard came to (or left) tile `key`.
    pub(crate) fn gif_tile_hovered(&mut self, key: MediaKey, on: bool) {
        if on {
            self.gif.tiles.hovered = Some(key);
        } else if self.gif.tiles.hovered == Some(key) {
            self.gif.tiles.hovered = None;
        }
        self.show_gifs();
    }

    /// No tile has the keyboard any more.
    pub(crate) fn gif_tile_unhovered(&mut self) {
        self.gif.tiles.hovered = None;
        self.show_gifs();
    }

    /// A tile's picture failed and its Retry was pressed.
    pub(crate) fn gif_tile_retry(&mut self, key: MediaKey) {
        let jobs = self.media.retry(key);
        self.run_media_jobs(jobs);
    }

    /// The tab's tiles changed (another chip, a search, a page) or it closed: what loads and
    /// plays follows at once, and the store forgets what is gone, so nothing more is fetched
    /// from GIPHY for it.
    pub(super) fn gif_tiles_changed(&mut self) {
        self.gif.tiles.tiles_changed();
        self.gif_tiles_settle();
    }

    /// The list or the grid moved. A step that changes nothing that draws runs no view.
    fn gif_places_changed(&mut self, tx: &mut Tx) {
        if !self.gif_tiles_settle() {
            tx.live_only();
        }
    }

    /// The tiles near the list load (asked for only when that set changed), the ones showing
    /// animate, and the next page is asked for once the last tiles come near. Returns whether
    /// anything that draws changed.
    fn gif_tiles_settle(&mut self) -> bool {
        let rects = self.gif_rects();
        let near = self.gif.tiles.near_list(&rects);
        let near_changed = near != self.gif.tiles.near;
        if near_changed {
            self.gif.tiles.near = near;
            self.want_media();
            self.gif_near_end();
        }
        let animation_changed = self.show_gifs();
        near_changed || animation_changed
    }

    /// One of the last tiles is near the list: GIPHY's next page is asked for.
    fn gif_near_end(&mut self) {
        let Some(account) = self.composer.shown().map(|(key, _)| key.account) else {
            return;
        };
        let count = self.gif_picker().map_or(0, |picker| picker.gifs().len());
        let near_end = self
            .gif
            .tiles
            .near
            .iter()
            .any(|at| *at + NEAR_END_TILES >= count);
        if !near_end {
            return;
        }
        let ask = self
            .gif_picker_mut()
            .and_then(crate::gif_picker::GifPicker::near_end);
        if let Some(ask) = ask {
            self.ask_giphy(account, ask);
        }
    }
}

#[cfg(test)]
mod tests {
    use papo_sdk::{AccountId, Gif, ServerAddress, Uuid};

    use super::{TilePlaces, fetchable};

    fn account() -> AccountId {
        AccountId::new(
            ServerAddress::parse("https://gif.invalid").unwrap().id(),
            Uuid::nil(),
        )
    }

    fn gif(n: u32) -> Gif {
        Gif {
            id: n.to_string(),
            url: format!("https://media.giphy.com/media/{n}/giphy.gif"),
            preview_width: 100,
            preview_height: 100,
            ..Gif::default()
        }
    }

    #[test]
    fn tiles_follow_the_grid_and_load_near_the_list_that_stayed_where_it_was() {
        let gifs: Vec<Gif> = (0..30).map(gif).collect();
        let mut places = TilePlaces {
            viewport: Some([0.0, 0.0, 308.0, 300.0]),
            grid: Some([0.0, 0.0, 308.0, 3000.0]),
            ..TilePlaces::default()
        };
        let rects = places.rects(account(), &gifs);
        assert_eq!(rects.len(), 30);
        // 100 pt squares, three across: rows of 104 pt; the list and one more list's height
        // below reach 600 pt: six rows.
        assert_eq!(places.near_list(&rects).len(), 18);
        // Another chip: the list and the grid did not move, so they report nothing.
        places.tiles_changed();
        assert_eq!(places.near_list(&places.rects(account(), &gifs)).len(), 18);
        places.grid = Some([0.0, -520.0, 308.0, 3000.0]);
        let rects = places.rects(account(), &gifs);
        assert!(
            places.near_list(&rects).contains(&29),
            "scrolled down: the last tiles"
        );
    }

    #[test]
    fn a_tile_animates_once_half_of_it_shows_and_one_gif_twice_counts_once() {
        let gifs = vec![gif(1), gif(2), gif(3), gif(1)];
        let places = TilePlaces {
            viewport: Some([0.0, 0.0, 308.0, 150.0]),
            grid: Some([0.0, 0.0, 308.0, 3000.0]),
            ..TilePlaces::default()
        };
        let rects = places.rects(account(), &gifs);
        // The first row shows whole; the fourth tile, the first GIF again, shows under half.
        let showing = places.showing(&rects, true);
        assert_eq!(showing.len(), 3, "{showing:?}");
        assert_eq!(places.showing(&rects, false), Vec::new());
    }

    #[test]
    fn only_giphy_tiles_are_fetched() {
        assert!(fetchable(&gif(1)));
        let elsewhere = Gif {
            url: "https://example.org/a.gif".to_owned(),
            ..Gif::default()
        };
        assert!(!fetchable(&elsewhere));
        let plain = Gif {
            url: "http://media.giphy.com/a.gif".to_owned(),
            ..Gif::default()
        };
        assert!(!fetchable(&plain));
    }
}
