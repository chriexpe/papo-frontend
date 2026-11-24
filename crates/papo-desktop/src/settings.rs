//! The settings panel (`docs/design.md` §8): grown from the account pill's
//! ⚙ (or Ctrl+,), a nav of pages with one-line summaries, search across
//! every row, sub-pages with ←, and rows that save as they change. D6a's
//! pages are Appearance and its two sub-pages; the others come with the
//! slices that make their rows work.
//!
//! Owns the panel's own state: the page shown, the search, a slider or the
//! colour field's value while it is dragged, the row whose save failed and
//! the colour field's selection. The settings themselves are
//! `appearance`'s; what a row's change does is the shell's
//! (`shell/settings.rs`).

use rune::TextEdit;

use crate::appearance::presets::Direction;
use crate::appearance::{Appearance, ShapeValue};
use crate::design::{Accent, Density, Theme};

pub(crate) mod colour_field;
mod controls;
pub(crate) mod rows;
mod search;
pub(crate) mod view;

pub(crate) use view::view;

/// The open panel's element key.
pub(crate) const SETTINGS_KEY: &str = "settings";
/// The search field's element key.
pub(crate) const SEARCH_KEY: &str = "settings-search";
/// The account pill's ⚙, which opens the panel and gets the keyboard back
/// when it closes.
pub(crate) const SETTINGS_BUTTON_KEY: &str = "account-settings";

/// A page of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PageId {
    Appearance,
    /// Appearance › Background colour.
    Background,
    /// Appearance › Shape and type.
    Shape,
}

impl PageId {
    /// Every page, nav pages first: what the search looks through.
    pub(crate) const ALL: [PageId; 3] = [PageId::Appearance, PageId::Background, PageId::Shape];

    /// The page a sub-page's ← goes back to.
    pub(crate) fn parent(self) -> Option<PageId> {
        match self {
            PageId::Appearance => None,
            PageId::Background | PageId::Shape => Some(PageId::Appearance),
        }
    }

    /// The nav page it belongs to.
    pub(crate) fn nav(self) -> PageId {
        self.parent().unwrap_or(self)
    }
}

/// One row, by what it sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum RowId {
    Theme,
    Background,
    Accent,
    Density,
    Grouped,
    Stacked,
    Shape,
    SystemFont,
    Glass,
    Field,
    Strength,
    Grain,
    Directions,
    Slider(ShapeValue),
}

impl RowId {
    /// Its element key, unique in the panel.
    pub(crate) fn key(self) -> String {
        format!("settings-row-{self:?}")
    }
}

/// What a row asks for; the shell applies it (`shell/settings.rs`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Change {
    Theme(Theme),
    Density(Density),
    Switch(RowId, bool),
    Accent(Accent),
    Direction(Direction),
    /// A slider at `value`; `done` when the drag ends or a key moved it,
    /// which saves it.
    Slider {
        row: RowId,
        value: f32,
        done: bool,
    },
    /// The colour field's colours, strength aside.
    Colours(colour_field::ColourChange),
}

impl Change {
    /// The row it belongs to, for its danger line.
    pub(crate) fn row(&self) -> RowId {
        match self {
            Change::Theme(_) => RowId::Theme,
            Change::Density(_) => RowId::Density,
            Change::Switch(row, _) | Change::Slider { row, .. } => *row,
            Change::Accent(_) => RowId::Accent,
            Change::Direction(_) => RowId::Directions,
            Change::Colours(_) => RowId::Field,
        }
    }
}

/// The colour field's own state: which colour the arrows move, which page
/// of presets shows, and the harmony arrangement the next press applies.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ColourState {
    pub(crate) selected: usize,
    pub(crate) preset_page: usize,
    pub(crate) harmony: usize,
    /// Where the colour being dragged sat when the press began: its drag
    /// moves it from there, however often the field is drawn meanwhile.
    pub(crate) drag_origin: Option<crate::design::BackgroundColour>,
}

/// A save that failed: the row that shows its danger line, and the
/// settings it was saving, which Try again saves as they were.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Failed {
    row: RowId,
    target: Appearance,
}

/// The open panel.
pub(crate) struct SettingsPanel {
    page: PageId,
    pub(crate) search: TextEdit,
    /// The save that failed, if the last one under its row did.
    failed: Option<Failed>,
    /// The settings while a slider or a colour is dragged: drawn, not
    /// saved, until the drag ends (§8: one write per change).
    draft: Option<Appearance>,
    pub(crate) colour: ColourState,
}

impl SettingsPanel {
    pub(crate) fn new() -> SettingsPanel {
        SettingsPanel {
            page: PageId::Appearance,
            search: TextEdit::new(""),
            failed: None,
            draft: None,
            colour: ColourState::default(),
        }
    }

    pub(crate) fn page(&self) -> PageId {
        self.page
    }

    /// Shows `page`, the search cleared so the page shows.
    pub(crate) fn go(&mut self, page: PageId) {
        self.page = page;
        self.failed = None;
        self.search = TextEdit::new("");
    }

    /// The search as typed, trimmed; empty when there is none.
    pub(crate) fn query(&self) -> &str {
        self.search.text.trim()
    }

    /// The row whose save failed, which shows the danger line.
    pub(crate) fn failed_row(&self) -> Option<RowId> {
        self.failed.map(|failed| failed.row)
    }

    /// The settings whose save failed, for Try again.
    pub(crate) fn failed_target(&self) -> Option<Appearance> {
        self.failed.map(|failed| failed.target)
    }

    /// The save of `target` under `row` worked, or did not.
    pub(crate) fn saved(&mut self, row: RowId, target: Appearance, ok: bool) {
        if ok {
            if self.failed_row() == Some(row) {
                self.failed = None;
            }
        } else {
            self.failed = Some(Failed { row, target });
        }
    }

    pub(crate) fn draft(&self) -> Option<&Appearance> {
        self.draft.as_ref()
    }

    /// A drag shows `appearance` without saving it.
    pub(crate) fn set_draft(&mut self, appearance: Appearance) {
        self.draft = Some(appearance);
    }

    /// The drag ended: what it showed, to be saved.
    pub(crate) fn take_draft(&mut self) -> Option<Appearance> {
        self.draft.take()
    }
}

#[cfg(test)]
mod tests;
