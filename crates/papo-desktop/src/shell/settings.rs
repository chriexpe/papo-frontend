//! The shell's side of the settings panel (`docs/design.md` §8): opening
//! it from the account pill's ⚙ or Ctrl+, and closing it, Escape's order
//! (a choice's menu closes itself first, then the search clears, then the
//! panel closes), moving between pages, the choice menus, and each row's
//! change applied to the Appearance settings: drawn while a slider or a
//! colour is dragged, saved when it lets go, and otherwise saved at once.
//! A save that fails keeps the earlier value and shows the row's danger
//! line, with Try again.
//!
//! Owns these transitions; the panel's look is `settings`', the settings'
//! storage `appearance`'s.

use rune::{Drag, Key, KeyEvent, Phase, TextEdit, Tx};

use super::Shell;
use crate::appearance::Appearance;
use crate::appearance::presets::{self, PRESETS, PRESETS_PER_PAGE};
use crate::design::{Background, BackgroundColour, Density, MAX_BACKGROUND_COLOURS, Theme};
use crate::menu::{MenuAction, MenuItem, OpenMenu, Placement};
use crate::settings::colour_field::{self, ColourChange};
use crate::settings::rows::{density_name, theme_name};
use crate::settings::{Change, PageId, RowId, SEARCH_KEY, SETTINGS_BUTTON_KEY, SettingsPanel};

/// What a choice's menu sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingChoice {
    Theme(Theme),
    Density(Density),
}

impl Shell {
    /// The account pill's ⚙ or Ctrl+,: opens the panel on Appearance, or
    /// closes it.
    pub(crate) fn toggle_settings(&mut self, tx: &mut Tx) {
        if self.settings.is_some() {
            self.close_settings();
            tx.focus(SETTINGS_BUTTON_KEY, true);
            return;
        }
        self.settings = Some(SettingsPanel::new());
        tx.focus(SEARCH_KEY, true);
    }

    /// Closes the panel; a drag still running is dropped, not saved.
    pub(crate) fn close_settings(&mut self) {
        self.settings = None;
    }

    /// Escape with nothing left to clear, or a press outside: the panel
    /// closes and the account pill's ⚙ gets the keyboard back.
    pub(crate) fn dismiss_settings(&mut self, tx: &mut Tx) {
        self.close_settings();
        tx.focus(SETTINGS_BUTTON_KEY, true);
    }

    #[cfg(test)]
    pub(crate) fn settings(&self) -> Option<&SettingsPanel> {
        self.settings.as_ref()
    }

    /// Escape inside the panel: the search clears first, then the panel
    /// closes. A choice's menu takes its own Escape before this.
    pub(crate) fn settings_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        if key.key != Key::Escape {
            return false;
        }
        let Some(panel) = self.settings.as_mut() else {
            return false;
        };
        if panel.query().is_empty() {
            // The panel's own dismissal closes it.
            return false;
        }
        panel.search = TextEdit::new("");
        tx.focus(SEARCH_KEY, true);
        true
    }

    /// The search field's text, for its editor.
    pub(crate) fn settings_search_mut(&mut self) -> &mut TextEdit {
        let Shell {
            settings,
            search_spare,
            ..
        } = self;
        settings.as_mut().map_or_else(
            || {
                *search_spare = TextEdit::new("");
                search_spare
            },
            |panel| &mut panel.search,
        )
    }

    /// The search's ✕.
    pub(crate) fn settings_clear_search(&mut self, tx: &mut Tx) {
        if let Some(panel) = self.settings.as_mut() {
            panel.search = TextEdit::new("");
            tx.focus(SEARCH_KEY, true);
        }
    }

    /// A nav row, a sub-page's row or ←: shows `page`.
    pub(crate) fn settings_page(&mut self, page: PageId, _tx: &mut Tx) {
        if let Some(panel) = self.settings.as_mut() {
            panel.go(page);
        }
    }

    /// A choice row: its menu under it, the current value ticked; `drawn`
    /// is the theme the window shows (the system's, until one is picked).
    pub(crate) fn settings_choice(&mut self, row: RowId, drawn: Theme, tx: &mut Tx) {
        let appearance = self.shown_appearance();
        let items: Vec<MenuItem> = match row {
            RowId::Theme => Theme::ALL
                .into_iter()
                .map(|theme| {
                    let chosen = theme == drawn;
                    MenuItem::plain(
                        &format!("settings-theme-{theme:?}"),
                        theme_name(theme),
                        MenuAction::Setting(SettingChoice::Theme(theme)),
                        false,
                    )
                    .with_radio(chosen)
                })
                .collect(),
            RowId::Density => Density::ALL
                .into_iter()
                .map(|density| {
                    MenuItem::plain(
                        &format!("settings-density-{density:?}"),
                        density_name(density),
                        MenuAction::Setting(SettingChoice::Density(density)),
                        false,
                    )
                    .with_radio(appearance.messages.density == density)
                })
                .collect(),
            _ => return,
        };
        self.nav.menu = Some(OpenMenu::new(
            Placement::BelowEnd(row.key()),
            row.key(),
            items,
        ));
        tx.focus(crate::menu::MENU_KEY, true);
    }

    /// A choice's menu item was picked.
    pub(crate) fn settings_chosen(&mut self, choice: SettingChoice, tx: &mut Tx) {
        let change = match choice {
            SettingChoice::Theme(theme) => Change::Theme(theme),
            SettingChoice::Density(density) => Change::Density(density),
        };
        self.settings_change(change, tx);
    }

    /// A row's change: drawn while a drag runs, else saved.
    pub(crate) fn settings_change(&mut self, change: Change, tx: &mut Tx) {
        let Some(panel) = self.settings.as_ref() else {
            return;
        };
        let current = panel
            .draft()
            .copied()
            .unwrap_or(*self.appearance.appearance());
        let colour = panel.colour;
        let (next, selected) = applied(&current, &change, colour.selected, colour.harmony);
        let Some(panel) = self.settings.as_mut() else {
            return;
        };
        panel.colour.selected = selected;
        if matches!(change, Change::Colours(ColourChange::Harmony)) {
            panel.colour.harmony += 1;
        }
        if dragging(&change) {
            panel.set_draft(next);
            return;
        }
        panel.take_draft();
        let saved = self.set_appearance(next, tx).is_ok();
        if let Some(panel) = self.settings.as_mut() {
            panel.saved(change, saved);
        }
    }

    /// Try again under a row whose save failed.
    pub(crate) fn settings_retry(&mut self, tx: &mut Tx) {
        let failed = self
            .settings
            .as_ref()
            .and_then(SettingsPanel::failed)
            .cloned();
        if let Some(change) = failed {
            self.settings_change(change, tx);
        }
    }

    /// The pointer on the colour field, the disc in its middle: a press
    /// grabs or adds a colour, a move drags it, letting go saves it.
    pub(crate) fn settings_field_drag(&mut self, drag: &mut Drag, tx: &mut Tx) {
        let centre = (drag.w / 2.0, drag.h / 2.0);
        let Some(panel) = self.settings.as_ref() else {
            return;
        };
        let current = panel
            .draft()
            .copied()
            .unwrap_or(*self.appearance.appearance());
        let at = (drag.x, drag.y);
        let change = match drag.phase {
            Phase::Down => {
                let (index, change) =
                    colour_field::pressed(&current, panel.colour.selected, at, centre);
                // An added colour is one more: the drag moves it from here.
                if let Some(panel) = self.settings.as_mut() {
                    panel.colour.selected = index;
                }
                match change {
                    ColourChange::Add(colour) => ColourChange::Add(colour),
                    moved => moved,
                }
            }
            Phase::Move => colour_field::dragged(panel.colour.selected, at, centre, false),
            Phase::Up => colour_field::dragged(panel.colour.selected, at, centre, true),
        };
        // An add saves at once only when no drag follows: it is drawn
        // while held, as a move is.
        let change = match (drag.phase, change) {
            (Phase::Down, ColourChange::Add(colour)) => {
                let mut added = current;
                let mut colours = current.background.colours().to_vec();
                colours.push(colour);
                added.background = Background::new(&colours, current.background.strength);
                if let Some(panel) = self.settings.as_mut() {
                    panel.set_draft(added);
                }
                return;
            }
            (_, change) => change,
        };
        self.settings_change(Change::Colours(change), tx);
    }

    /// A colour on the field dragged: from where it sat at the press
    /// (`now`, the colour as drawn when the press began), by how far the
    /// pointer has moved; it takes the arrows and − from then on.
    pub(crate) fn settings_colour_drag(
        &mut self,
        index: usize,
        now: BackgroundColour,
        drag: &mut Drag,
        tx: &mut Tx,
    ) {
        let Some(panel) = self.settings.as_mut() else {
            return;
        };
        if drag.phase == Phase::Down {
            panel.colour.selected = index;
            panel.colour.drag_origin = Some(now);
            return;
        }
        let origin = panel.colour.drag_origin.unwrap_or(now);
        if drag.phase == Phase::Up {
            panel.colour.drag_origin = None;
        }
        let change = colour_field::moved_by(index, origin, (drag.dx, drag.dy), drag.phase);
        self.settings_change(Change::Colours(change), tx);
    }

    /// ‹ or › beside the presets.
    pub(crate) fn settings_preset_page(&mut self, page: usize) {
        if let Some(panel) = self.settings.as_mut() {
            let pages = PRESETS.len().div_ceil(PRESETS_PER_PAGE);
            panel.colour.preset_page = page.min(pages - 1);
        }
    }

    /// The settings drawn: a drag's draft while one runs, else the saved
    /// ones.
    pub(crate) fn shown_appearance(&self) -> Appearance {
        self.settings
            .as_ref()
            .and_then(SettingsPanel::draft)
            .copied()
            .unwrap_or(*self.appearance.appearance())
    }
}

/// Whether `change` is a drag still running: drawn, not saved.
fn dragging(change: &Change) -> bool {
    matches!(
        change,
        Change::Slider { done: false, .. }
            | Change::Colours(ColourChange::Move { done: false, .. })
    )
}

/// `current` with `change` applied, and the colour the field's arrows act
/// on after it.
fn applied(
    current: &Appearance,
    change: &Change,
    selected: usize,
    harmony: usize,
) -> (Appearance, usize) {
    let mut next = *current;
    let mut selected = selected;
    match change {
        Change::Theme(theme) => next.theme = Some(*theme),
        Change::Density(density) => next.messages.density = *density,
        Change::Switch(RowId::Grouped, on) => next.messages.grouped = *on,
        Change::Switch(RowId::Stacked, on) => next.messages.stacked = *on,
        Change::Switch(RowId::SystemFont, on) => next.system_font = *on,
        Change::Accent(accent) => next.accent = *accent,
        Change::Direction(direction) => next.shape = direction.shape(),
        Change::Slider {
            row: RowId::Strength,
            value,
            ..
        } => {
            let colours = current.background.colours().to_vec();
            next.background = Background::new(&colours, *value);
        }
        Change::Slider {
            row: RowId::Glass,
            value,
            ..
        } => next.glass = value.clamp(0.0, 1.0),
        Change::Slider {
            row: RowId::Grain,
            value,
            ..
        } => next.grain = value.clamp(0.0, 1.0),
        Change::Slider {
            row: RowId::Slider(value_of),
            value,
            ..
        } => next.shape = current.shape.with(*value_of, *value),
        Change::Switch(..) | Change::Slider { .. } => {}
        Change::Colours(colour) => {
            let (background, now) =
                colours_applied(&current.background, *colour, selected, harmony);
            next.background = background;
            selected = now;
        }
    }
    (next, selected)
}

/// `background` with the field's `change`, and the selected colour after.
fn colours_applied(
    background: &Background,
    change: ColourChange,
    selected: usize,
    harmony: usize,
) -> (Background, usize) {
    let strength = background.strength;
    let mut colours: Vec<BackgroundColour> = background.colours().to_vec();
    let mut selected = selected;
    match change {
        ColourChange::Move { index, colour, .. } => {
            if let Some(slot) = colours.get_mut(index) {
                *slot = colour;
                selected = index;
            }
        }
        ColourChange::Add(colour) if colours.len() < MAX_BACKGROUND_COLOURS => {
            colours.push(colour);
            selected = colours.len() - 1;
        }
        ColourChange::Add(_) => {}
        ColourChange::Remove => {
            if selected < colours.len() {
                colours.remove(selected);
            }
            selected = selected.min(colours.len().saturating_sub(1));
        }
        ColourChange::Harmony => {
            return (presets::harmony(background, harmony), selected);
        }
        ColourChange::Preset(id) => {
            let preset = PRESETS.iter().find(|preset| preset.id == id);
            return (preset.map_or(*background, presets::Preset::background), 0);
        }
    }
    (Background::new(&colours, strength), selected)
}
