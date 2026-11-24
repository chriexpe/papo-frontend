//! The settings panel (`docs/design.md` §8): grown from the account pill's
//! ⚙ (or Ctrl+,), a nav of pages with one-line summaries, search across
//! every row, sub-pages with ←, and rows that save as they change. The App
//! pages are Appearance and Conversations, each with its sub-pages; the
//! others come with the slices that make their rows work.
//!
//! Owns the panel's own state: the page shown, the search, a slider or the
//! colour field's value while it is dragged, the row whose save failed and
//! the colour field's selection. The settings themselves are their
//! stores' (`appearance`, `conversation_settings`); what a row's change does
//! is the shell's (`shell/settings.rs`).

use rune::TextEdit;

use crate::appearance::presets::Direction;
use crate::appearance::{Appearance, ShapeValue};
use crate::conversation_settings::ConversationSettings;
use crate::design::{Accent, BackgroundColour, Density, Theme};

pub(crate) mod colour_field;
mod colour_tools;
mod controls;
pub(crate) mod rows;
mod search;
mod slider;
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
    Conversations,
    /// Conversations › Message highlights.
    Highlights,
}

impl PageId {
    /// Every page, nav pages first and in the nav's order: what the search
    /// looks through.
    pub(crate) const ALL: [PageId; 5] = [
        PageId::Appearance,
        PageId::Conversations,
        PageId::Background,
        PageId::Shape,
        PageId::Highlights,
    ];

    /// The pages in the nav, in order.
    pub(crate) const NAV: [PageId; 2] = [PageId::Appearance, PageId::Conversations];

    /// The page a sub-page's ← goes back to.
    pub(crate) fn parent(self) -> Option<PageId> {
        match self {
            PageId::Appearance | PageId::Conversations => None,
            PageId::Background | PageId::Shape => Some(PageId::Appearance),
            PageId::Highlights => Some(PageId::Conversations),
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
    EnterSends,
    ShowTimes,
    ShowAvatars,
    OpenAtNewest,
    Hourglass,
    ChannelDescription,
    Highlights,
    MentionHue,
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
    /// A hue swatch picked, in OKLCH degrees.
    Hue(RowId, f32),
}

impl Change {
    /// The row it belongs to, for its danger line.
    pub(crate) fn row(&self) -> RowId {
        match self {
            Change::Theme(_) => RowId::Theme,
            Change::Density(_) => RowId::Density,
            Change::Switch(row, _) | Change::Slider { row, .. } | Change::Hue(row, _) => *row,
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
}

/// A drag in the panel (§8, Saving): what it shows is drawn, not saved,
/// until it lets go.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Held {
    /// Nothing held.
    #[default]
    Idle,
    /// A colour on the field pressed, not moved yet: where it sat, which
    /// its drag moves it from however often the field is drawn meanwhile.
    Colour { origin: BackgroundColour },
    /// A slider, the field or a colour dragged: the settings as drawn,
    /// and a colour's `origin` while it is that colour's own drag.
    Drawing {
        draft: Appearance,
        origin: Option<BackgroundColour>,
    },
}

/// What happens to a drag.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum HeldEvent {
    /// A colour's own drag pressed it where it sits.
    ColourPressed(BackgroundColour),
    /// The drag draws these settings.
    Drew(Appearance),
    /// It let go (the caller saves what it ends on) or was cancelled.
    Ended,
}

impl Held {
    /// The drag after `event`.
    pub(crate) fn step(self, event: HeldEvent) -> Held {
        match (self, event) {
            (_, HeldEvent::ColourPressed(origin)) => Held::Colour { origin },
            (Held::Colour { origin }, HeldEvent::Drew(draft)) => Held::Drawing {
                draft,
                origin: Some(origin),
            },
            (Held::Drawing { origin, .. }, HeldEvent::Drew(draft)) => {
                Held::Drawing { draft, origin }
            }
            (Held::Idle, HeldEvent::Drew(draft)) => Held::Drawing {
                draft,
                origin: None,
            },
            (_, HeldEvent::Ended) => Held::Idle,
        }
    }

    /// The settings the drag draws, while it draws any.
    pub(crate) fn draft(&self) -> Option<&Appearance> {
        match self {
            Held::Drawing { draft, .. } => Some(draft),
            Held::Idle | Held::Colour { .. } => None,
        }
    }

    /// Where the colour being dragged sat at the press.
    pub(crate) fn origin(&self) -> Option<BackgroundColour> {
        match *self {
            Held::Colour { origin } => Some(origin),
            Held::Drawing { origin, .. } => origin,
            Held::Idle => None,
        }
    }
}

/// What a row saves: the settings its store holds after the change.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Target {
    Appearance(Appearance),
    Conversations(ConversationSettings),
}

/// A save that failed: the row that shows its danger line, and the
/// settings it was saving, which Try again saves as they were.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Failed {
    row: RowId,
    target: Target,
}

/// The open panel.
pub(crate) struct SettingsPanel {
    page: PageId,
    pub(crate) search: TextEdit,
    /// The save that failed, if the last one under its row did.
    failed: Option<Failed>,
    /// A drag, drawn but not saved until it ends (§8: one write per
    /// change).
    held: Held,
    pub(crate) colour: ColourState,
}

impl SettingsPanel {
    pub(crate) fn new() -> SettingsPanel {
        SettingsPanel {
            page: PageId::Appearance,
            search: TextEdit::new(""),
            failed: None,
            held: Held::Idle,
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
    pub(crate) fn failed_target(&self) -> Option<Target> {
        self.failed.map(|failed| failed.target)
    }

    /// The save of `target` under `row` worked, or did not.
    pub(crate) fn saved(&mut self, row: RowId, target: Target, ok: bool) {
        if ok {
            if self.failed_row() == Some(row) {
                self.failed = None;
            }
        } else {
            self.failed = Some(Failed { row, target });
        }
    }

    /// The settings a drag draws, while one draws any.
    pub(crate) fn draft(&self) -> Option<&Appearance> {
        self.held.draft()
    }

    /// Where the colour being dragged sat at the press.
    pub(crate) fn drag_origin(&self) -> Option<BackgroundColour> {
        self.held.origin()
    }

    /// The drag moves on `event`.
    pub(crate) fn held(&mut self, event: HeldEvent) {
        self.held = self.held.step(event);
    }
}

#[cfg(test)]
mod tests;
