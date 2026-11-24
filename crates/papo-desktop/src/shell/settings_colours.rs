//! The Background colour field's input in the shell (`docs/design.md` §4):
//! a press on the field grabs or adds a colour, a colour's own drag moves
//! it from where it sat, ‹ › page the presets, and what each asks for
//! applied to the background's colours.
//!
//! Owns those handlers and the colour arithmetic; saving is
//! `shell/settings.rs`'s (`settings_change`), the field's look and geometry
//! `settings::colour_field`'s.

use rune::{Drag, Phase, Tx};

use super::Shell;
use crate::appearance::presets::{self, PRESETS, PRESETS_PER_PAGE};
use crate::design::{Background, BackgroundColour, MAX_BACKGROUND_COLOURS};
use crate::settings::Change;
use crate::settings::colour_field::{self, ColourChange};

impl Shell {
    /// The pointer on the colour field, the disc in its middle: a press
    /// grabs or adds a colour, a move drags it, letting go saves it.
    pub(crate) fn settings_field_drag(&mut self, drag: &mut Drag, tx: &mut Tx) {
        if drag.phase == Phase::Up && drag.cancelled() {
            self.settings_drag_cancelled();
            return;
        }
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
                change
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
        if drag.phase == Phase::Up && drag.cancelled() {
            self.settings_drag_cancelled();
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
}

/// `background` with the field's `change`, and the selected colour after.
pub(super) fn colours_applied(
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
