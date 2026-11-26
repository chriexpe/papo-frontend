//! The picker's GIF tab (`docs/design.md` §16.3, §16.5): what it shows (a chip: Favorites,
//! Recent, Trending, or a search), the pages GIPHY answered, whether GIPHY can be asked at all,
//! and the tile the keyboard is on.
//!
//! Pure: the shell asks the SDK for what [`Ask`] names and hands the answers back; an answer to
//! an older ask is dropped. Favorites and Recent are the account's library as it stood when the
//! tab opened (§16.4: starring inside the picker never moves a tile under the pointer). The
//! columns are `masonry`'s, the drawing `view`'s.

use papo_sdk::Gif;
use rune::TextEdit;

pub(crate) mod masonry;
pub(crate) mod view;

/// Recent shows the account's last sent GIFs, at most this many (§16.4).
const RECENT: usize = 8;
/// How long typing must pause before GIPHY is searched, in seconds (§16.3).
pub(crate) const SEARCH_AFTER_S: f64 = 0.3;
/// The longest search GIPHY takes, in characters.
const MAX_QUERY_CHARS: usize = 50;

/// What the tab shows: a chip, or what the search field holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shelf {
    Favorites,
    Recent,
    Trending,
    Search(String),
}

/// GIPHY as the tab found it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Giphy {
    /// This build has no key: no search, no Trending, no mark (§16.3).
    NoKey,
    /// It answers, or hasn't been asked yet.
    Ready,
    /// The last ask failed: "Couldn't reach GIPHY." and Try again.
    Unreachable,
}

/// Where Trending's or a search's next page stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Paging {
    /// Nothing on its way.
    Idle,
    /// A page is coming: placeholders and "Loading more" (the first page: placeholders only).
    Loading,
}

/// A page to ask GIPHY for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ask {
    /// Its tag: only the newest ask's answer is taken.
    pub(crate) tag: u64,
    /// The search; none for Trending.
    pub(crate) query: Option<String>,
    pub(crate) offset: u32,
}

/// A page GIPHY answered: its GIFs and where the next one starts, if there is one.
pub(crate) struct Page {
    pub(crate) gifs: Vec<Gif>,
    pub(crate) next_offset: Option<u32>,
}

/// The open GIF tab.
pub(crate) struct GifPicker {
    /// Its search field.
    pub(crate) search: TextEdit,
    pub(crate) shelf: Shelf,
    pub(crate) giphy: Giphy,
    favorites: Vec<Gif>,
    recent: Vec<Gif>,
    /// GIPHY's trending searches, as chips after Trending.
    pub(crate) trending_searches: Vec<String>,
    /// Trending's or the search's GIFs so far, in GIPHY's order.
    pages: Vec<Gif>,
    next_offset: Option<u32>,
    pub(crate) paging: Paging,
    /// The newest ask's tag.
    asked: u64,
    /// When the search, if the typing has paused, is due (the window's clock, in seconds).
    pub(crate) search_due_s: Option<f64>,
    /// The tile the keyboard is on (an index into [`GifPicker::gifs`]); none while it is in the
    /// search or the chips.
    pub(crate) cursor: Option<usize>,
    /// Bumped when what the tiles show changes: the columns' version.
    pub(crate) version: u64,
}

impl GifPicker {
    /// The tab for the account's `favorites` and `recent` GIFs, with GIPHY when this build has a
    /// key (`keyed`). It opens on Favorites when there are any, else on Trending (else, without
    /// GIPHY, on Favorites still, which says how to add one).
    pub(crate) fn new(keyed: bool, favorites: Vec<Gif>, mut recent: Vec<Gif>) -> GifPicker {
        recent.truncate(RECENT);
        let shelf = if favorites.is_empty() && keyed {
            Shelf::Trending
        } else {
            Shelf::Favorites
        };
        GifPicker {
            search: TextEdit::new(""),
            shelf,
            giphy: if keyed { Giphy::Ready } else { Giphy::NoKey },
            favorites,
            recent,
            trending_searches: Vec::new(),
            pages: Vec::new(),
            next_offset: None,
            paging: Paging::Idle,
            asked: 0,
            search_due_s: None,
            cursor: None,
            version: 0,
        }
    }

    /// The tab with its asks tagged from `first`: an earlier opening's answers carry tags
    /// below it and are never taken.
    #[must_use]
    pub(crate) fn tags_from(mut self, first: u64) -> GifPicker {
        self.asked = first;
        self
    }

    /// The GIFs the tab shows, in order.
    pub(crate) fn gifs(&self) -> &[Gif] {
        match self.shelf {
            Shelf::Favorites => &self.favorites,
            Shelf::Recent => &self.recent,
            Shelf::Trending | Shelf::Search(_) => &self.pages,
        }
    }

    /// How many favorites the Favorites chip counts.
    pub(crate) fn favorite_count(&self) -> usize {
        self.favorites.len()
    }

    /// Whether `shelf` is GIPHY's (Trending, a search): its pages are asked for.
    fn from_giphy(shelf: &Shelf) -> bool {
        matches!(shelf, Shelf::Trending | Shelf::Search(_))
    }

    /// A chip was chosen; a search chip fills the search field. Returns the first page to ask
    /// for, if the shelf is GIPHY's.
    pub(crate) fn choose(&mut self, chip: Shelf) -> Option<Ask> {
        if let Shelf::Search(query) = &chip {
            self.search = TextEdit::new(query);
            self.search.set_caret(self.search.text.len());
        } else if !self.search.text.is_empty() {
            self.search = TextEdit::new("");
        }
        self.search_due_s = None;
        self.show(chip)
    }

    /// Shows `shelf` from its start; GIPHY's begin with a page asked for.
    fn show(&mut self, shown: Shelf) -> Option<Ask> {
        // Another shelf asks GIPHY again, even after it couldn't be reached.
        if self.giphy == Giphy::Unreachable {
            self.giphy = Giphy::Ready;
        }
        self.shelf = shown;
        self.cursor = None;
        self.pages.clear();
        self.next_offset = None;
        self.paging = Paging::Idle;
        self.version = self.version.wrapping_add(1);
        self.next_ask(0)
    }

    /// The search field changed at `clock_s`: the search waits for a pause in typing. An
    /// emptied field goes back to Trending.
    pub(crate) fn typed(&mut self, clock_s: f64) -> Option<Ask> {
        if self.giphy == Giphy::NoKey {
            return None;
        }
        if self.query().is_none() {
            self.search_due_s = None;
            if matches!(self.shelf, Shelf::Search(_)) {
                return self.show(Shelf::Trending);
            }
            return None;
        }
        self.search_due_s = Some(clock_s + SEARCH_AFTER_S);
        None
    }

    /// The window's clock reached `clock_s`: a search whose pause is over is asked for.
    pub(crate) fn due(&mut self, clock_s: f64) -> Option<Ask> {
        let due_s = self.search_due_s?;
        if clock_s + 1e-3 < due_s {
            return None;
        }
        self.search_due_s = None;
        let query = self.query()?;
        if self.shelf == Shelf::Search(query.clone()) {
            return None;
        }
        self.show(Shelf::Search(query))
    }

    /// The search as GIPHY takes it: trimmed, at most its longest; none when empty.
    fn query(&self) -> Option<String> {
        let query: String = self
            .search
            .text
            .trim()
            .chars()
            .take(MAX_QUERY_CHARS)
            .collect();
        (!query.is_empty()).then_some(query)
    }

    /// The tiles near the end show: GIPHY's next page is asked for, when there is one and none
    /// is on its way.
    pub(crate) fn near_end(&mut self) -> Option<Ask> {
        if self.paging != Paging::Idle || self.giphy != Giphy::Ready {
            return None;
        }
        let offset = self.next_offset?;
        self.next_ask(offset)
    }

    /// Try again, after GIPHY couldn't be reached: the shelf's page is asked for again.
    pub(crate) fn try_again(&mut self) -> Option<Ask> {
        if self.giphy != Giphy::Unreachable {
            return None;
        }
        self.giphy = Giphy::Ready;
        let offset = if self.pages.is_empty() {
            0
        } else {
            self.next_offset?
        };
        self.next_ask(offset)
    }

    /// An ask for the shelf's page at `offset`, when the shelf is GIPHY's and GIPHY can be asked.
    fn next_ask(&mut self, offset: u32) -> Option<Ask> {
        if !Self::from_giphy(&self.shelf) || self.giphy != Giphy::Ready {
            return None;
        }
        self.asked = self.asked.wrapping_add(1);
        self.paging = Paging::Loading;
        Some(Ask {
            tag: self.asked,
            query: match &self.shelf {
                Shelf::Search(query) => Some(query.clone()),
                _ => None,
            },
            offset,
        })
    }

    /// GIPHY answered ask `tag`: its page joins the shelf, or GIPHY is unreachable. An older
    /// ask's answer changes nothing. Returns whether the tab changed.
    pub(crate) fn landed(&mut self, tag: u64, page: Option<Page>) -> bool {
        if tag != self.asked || self.paging != Paging::Loading {
            return false;
        }
        self.paging = Paging::Idle;
        match page {
            Some(page) => {
                self.pages.extend(page.gifs);
                self.next_offset = page.next_offset;
            }
            None => self.giphy = Giphy::Unreachable,
        }
        self.version = self.version.wrapping_add(1);
        true
    }

    /// GIPHY's trending searches arrived.
    pub(crate) fn searches_landed(&mut self, searches: Vec<String>) {
        self.trending_searches = searches;
    }

    /// Whether the tab waits for its first page: placeholder tiles only.
    pub(crate) fn first_page_loading(&self) -> bool {
        self.paging == Paging::Loading && self.pages.is_empty()
    }

    /// Whether more of the shelf is coming: placeholders and "Loading more" under the tiles.
    pub(crate) fn more_loading(&self) -> bool {
        self.paging == Paging::Loading && !self.pages.is_empty()
    }

    /// ↑ or ↓: the keyboard moves to the tile above or below in its column of the masonry;
    /// up from a column's first tile is the search (none). Into the tiles from the search, the
    /// first tile.
    pub(crate) fn step_vertical(&mut self, down: bool) -> Option<usize> {
        let Some(at) = self.cursor else {
            return if down { self.step(1) } else { None };
        };
        // The columns' order is the same at any width: heights scale alike.
        let columns = masonry::columns(self.gifs(), 100.0);
        let Some((column, row)) = columns.iter().enumerate().find_map(|(column, tiles)| {
            tiles
                .iter()
                .position(|tile| tile.at == at)
                .map(|row| (column, row))
        }) else {
            return self.cursor;
        };
        self.cursor = if down {
            columns[column]
                .get(row + 1)
                .map_or(Some(at), |tile| Some(tile.at))
        } else {
            row.checked_sub(1).map(|above| columns[column][above].at)
        };
        self.cursor
    }

    /// Home or End: the first tile, or the last.
    pub(crate) fn step_to_end(&mut self, last: bool) -> Option<usize> {
        let count = self.gifs().len();
        if count > 0 && self.cursor.is_some() {
            self.cursor = Some(if last { count - 1 } else { 0 });
        }
        self.cursor
    }

    /// The keyboard moves over the tiles in GIPHY's order; none is the search.
    pub(crate) fn step(&mut self, by: isize) -> Option<usize> {
        let count = self.gifs().len();
        if count == 0 {
            self.cursor = None;
            return None;
        }
        self.cursor = match self.cursor {
            None if by > 0 => Some(0),
            None => None,
            Some(at) => {
                let next = isize::try_from(at).unwrap_or(0) + by;
                if next < 0 {
                    None
                } else {
                    Some(usize::try_from(next).unwrap_or(0).min(count - 1))
                }
            }
        };
        self.cursor
    }

    /// The GIF the keyboard is on.
    pub(crate) fn chosen(&self) -> Option<&Gif> {
        self.gifs().get(self.cursor?)
    }
}

#[cfg(test)]
mod tests;
