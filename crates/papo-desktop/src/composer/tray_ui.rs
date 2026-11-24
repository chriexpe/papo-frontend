//! The tray's interaction state (`docs/design.md` §14.2): the tile the
//! keyboard is on, and a tile being dragged to a new place, with where each
//! tile was last drawn so a drag knows where it would land.
//!
//! Owns no tiles: those are the draft's (`tray`). The shell steps it from
//! the tray's handlers (`shell/tray.rs`) and the view reads it.

use std::collections::HashMap;

use rune::Live;

use super::tray::TileId;

/// What the pointer is doing to the tray.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum Gesture {
    #[default]
    Idle,
    /// Tile `id` is held and moved; it would land before the tile at
    /// `slot` (the tray's length: at the end).
    Dragging { id: TileId, slot: usize },
}

/// The shown tray's interaction state.
pub(crate) struct TrayUi {
    /// Whether the tray has the keyboard.
    pub(crate) focused: bool,
    /// The tile the keyboard is on while the tray has it.
    pub(crate) current: Option<TileId>,
    pub(crate) gesture: Gesture,
    /// Where each tile was last drawn: left edge and width, window points.
    spans: HashMap<TileId, (f32, f32)>,
    /// How far the tray is scrolled sideways, in points, as its viewport
    /// last reported it.
    pub(crate) scrolled: Live<f32>,
}

impl Default for TrayUi {
    fn default() -> TrayUi {
        TrayUi {
            focused: false,
            current: None,
            gesture: Gesture::Idle,
            spans: HashMap::new(),
            scrolled: Live::new(0.0),
        }
    }
}

impl std::fmt::Debug for TrayUi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrayUi")
            .field("focused", &self.focused)
            .field("current", &self.current)
            .field("gesture", &self.gesture)
            .finish_non_exhaustive()
    }
}

impl TrayUi {
    /// Tile `id` was drawn at `left` with `width`.
    pub(crate) fn drawn(&mut self, id: TileId, left: f32, width: f32) {
        self.spans.insert(id, (left, width));
    }

    /// Whether a tile is being dragged.
    pub(crate) fn is_dragging(&self) -> bool {
        matches!(self.gesture, Gesture::Dragging { .. })
    }

    /// Where tile `id` was last drawn from, in window points.
    pub(crate) fn left_of(&self, id: TileId) -> Option<f32> {
        self.spans.get(&id).map(|(left, _)| *left)
    }

    /// The tray got or lost the keyboard: it lands on the current tile, or
    /// the last one (⇧Tab from the field lands there).
    pub(crate) fn focus_changed(&mut self, focused: bool, ids: &[TileId]) {
        self.focused = focused;
        if focused && self.current_in(ids).is_none() {
            self.current = ids.last().copied();
        }
    }

    /// The tile the keyboard is on, when it is still in `tiles`.
    pub(crate) fn current_in(&self, ids: &[TileId]) -> Option<TileId> {
        self.current.filter(|id| ids.contains(id))
    }

    /// ← or →: the keyboard moves to the tile `step` places away, stopping
    /// at the ends.
    pub(crate) fn step(&mut self, ids: &[TileId], step: isize) {
        let at = self
            .current_in(ids)
            .and_then(|id| ids.iter().position(|other| *other == id))
            .unwrap_or(0);
        let to = at
            .saturating_add_signed(step)
            .min(ids.len().saturating_sub(1));
        self.current = ids.get(to).copied();
    }

    /// The pointer drags tile `id`, now at window x `pointer_x`: where it
    /// would land among `tiles`, by the drawn tiles' middles.
    pub(crate) fn dragged(&mut self, id: TileId, pointer_x: f32, ids: &[TileId]) {
        let slot = ids
            .iter()
            .take_while(|other| {
                self.spans
                    .get(other)
                    .is_some_and(|(left, width)| left + width / 2.0 < pointer_x)
            })
            .count();
        self.gesture = Gesture::Dragging { id, slot };
    }

    /// The drag ended: the tile and the place it lands in the tray's order,
    /// none when it was not dragged.
    pub(crate) fn dropped(&mut self, ids: &[TileId]) -> Option<(TileId, usize)> {
        let Gesture::Dragging { id, slot } = std::mem::take(&mut self.gesture) else {
            return None;
        };
        let from = ids.iter().position(|other| *other == id)?;
        // The tile leaves its place first, so a slot past it is one less.
        let to = if slot > from { slot - 1 } else { slot };
        Some((id, to))
    }

    /// The drag was cancelled (Escape, or the pointer was taken away).
    pub(crate) fn cancel_drag(&mut self) {
        self.gesture = Gesture::Idle;
    }

    /// Forgets what was drawn of tiles no longer in `tiles`.
    pub(crate) fn retain(&mut self, ids: &[TileId]) {
        self.spans.retain(|id, _| ids.contains(id));
    }
}

#[cfg(test)]
mod tests {
    //! Where the keyboard goes and where a dragged tile lands.
    use super::*;

    const IDS: [TileId; 3] = [TileId(1), TileId(2), TileId(3)];

    /// Three tiles drawn side by side, 72 pt wide, 8 pt apart.
    fn drawn() -> TrayUi {
        let mut ui = TrayUi::default();
        for (n, id) in IDS.iter().enumerate() {
            #[expect(clippy::cast_precision_loss, reason = "three tiles")]
            ui.drawn(*id, 80.0 * n as f32, 72.0);
        }
        ui
    }

    #[test]
    fn the_keyboard_lands_on_the_last_tile_and_stops_at_the_ends() {
        let mut ui = TrayUi::default();
        ui.focus_changed(true, &IDS);
        assert_eq!(ui.current, Some(TileId(3)));
        ui.step(&IDS, 1);
        assert_eq!(ui.current, Some(TileId(3)));
        ui.step(&IDS, -5);
        assert_eq!(ui.current, Some(TileId(1)));
    }

    #[test]
    fn a_tile_dragged_past_the_next_middle_lands_after_it() {
        let mut ui = drawn();
        ui.dragged(TileId(1), 120.0, &IDS);
        assert_eq!(
            ui.gesture,
            Gesture::Dragging {
                id: TileId(1),
                slot: 2
            }
        );
        assert_eq!(ui.dropped(&IDS), Some((TileId(1), 1)));
        assert_eq!(ui.gesture, Gesture::Idle);
    }

    #[test]
    fn a_tile_dragged_to_the_front_lands_first() {
        let mut ui = drawn();
        ui.dragged(TileId(3), 10.0, &IDS);
        assert_eq!(ui.dropped(&IDS), Some((TileId(3), 0)));
    }

    #[test]
    fn a_drop_without_a_drag_moves_nothing() {
        assert_eq!(TrayUi::default().dropped(&IDS), None);
    }
}
