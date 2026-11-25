//! The tray's handlers (`docs/design.md` §14.2): the keyboard on the tray
//! (←/→ between tiles, Alt+←/→ to move one, Delete or Backspace to remove,
//! the menu key, Escape back to the field), a tile dragged to a new place,
//! and the tile menu.
//!
//! Owns the steps; the state is `TrayUi` and the draft's tray.

use rune::{Drag, Key, KeyEvent, Mods, Phase, PointerPress, Tx, WheelEvent};

use super::Shell;
use crate::composer::COMPOSER_KEY;
use crate::composer::tray::TileId;
use crate::composer::tray_view::{TRAY_KEY, tile_key_of};
use crate::icons;
use crate::menu::{MenuAction, MenuItem, OpenMenu, Placement};
use crate::strings;

/// How far a press must travel, in points, before it drags a tile.
const DRAG_START_PT: f32 = 4.0;

impl Shell {
    /// The shown tray's tile identities, in order.
    fn tray_ids(&self) -> Vec<TileId> {
        self.composer
            .shown()
            .map(|(_, draft)| draft.tray.ids())
            .unwrap_or_default()
    }

    /// The tray got or lost the keyboard; the tile it lands on scrolls into
    /// view.
    pub(crate) fn tray_focus_changed(&mut self, focused: bool, tx: &mut Tx) {
        let ids = self.tray_ids();
        self.tray_ui.focus_changed(focused, &ids);
        self.reveal_current(tx);
    }

    /// Keeps the tile the keyboard is on in view, scrolling the tray only.
    fn reveal_current(&self, tx: &mut Tx) {
        if let Some(id) = self.tray_ui.current {
            tx.scroll_into_view_in(TRAY_KEY, tile_key_of(id));
        }
    }

    /// The wheel over the tray: mostly-vertical motion scrolls it sideways
    /// (down moves on towards the last tile). Sideways motion is left to
    /// the viewport.
    pub(crate) fn tray_wheel(&mut self, wheel: WheelEvent, tx: &mut Tx) -> bool {
        if wheel.dy.abs() <= wheel.dx.abs() {
            return false;
        }
        let to = (self.tray_ui.scrolled.get() - wheel.dy).max(0.0);
        tx.scroll_to_x(TRAY_KEY, to);
        true
    }

    /// A key on the tray. Enter and Space open the editor on the current
    /// tile; Tab leaves as it always does.
    pub(crate) fn tray_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let ids = self.tray_ids();
        // A tile removed by the pointer leaves no current one: the keys
        // then start from the last.
        let Some(current) = self.tray_ui.current_in(&ids).or(ids.last().copied()) else {
            return false;
        };
        self.tray_ui.current = Some(current);
        let bare = key.mods == Mods::default();
        let alt_only = key.mods.alt && !key.mods.ctrl && !key.mods.shift && !key.mods.logo;
        match key.key {
            Key::Left | Key::Right if alt_only => {
                let step = if key.key == Key::Left { -1 } else { 1 };
                if let Some((draft_key, _)) = self.composer.shown() {
                    let effects = self.composer.shift_tile(draft_key, current, step);
                    self.run_composer(effects, Some(tx));
                }
                self.reveal_current(tx);
                true
            }
            Key::Left if bare => {
                self.tray_ui.step(&ids, -1);
                self.reveal_current(tx);
                true
            }
            Key::Right if bare => {
                self.tray_ui.step(&ids, 1);
                self.reveal_current(tx);
                true
            }
            Key::Delete | Key::Backspace if bare => {
                let next = self.remove_tile(current, tx);
                self.tray_ui.current = next;
                self.reveal_current(tx);
                if next.is_none() {
                    // The tray is gone: the field takes the keyboard back.
                    tx.focus(COMPOSER_KEY, true);
                }
                true
            }
            Key::Escape if bare => {
                tx.focus(COMPOSER_KEY, true);
                true
            }
            Key::Enter if bare => {
                self.edit_tile(current);
                true
            }
            Key::Text(ref text) if bare && text == " " => {
                self.edit_tile(current);
                true
            }
            _ => false,
        }
    }

    /// A press on tile `id` moved: past a few points it drags the tile,
    /// and its release puts it where the accent line shows.
    pub(crate) fn tray_drag(&mut self, id: TileId, drag: &mut Drag, tx: &mut Tx) {
        let ids = self.tray_ids();
        match drag.phase {
            Phase::Down => {}
            Phase::Move => {
                let dragging = self.tray_ui.is_dragging()
                    || drag.dx.abs() > DRAG_START_PT
                    || drag.dy.abs() > DRAG_START_PT;
                if let (true, Some(left)) = (dragging, self.tray_ui.left_of(id)) {
                    self.tray_ui.dragged(id, left + drag.x, &ids);
                }
            }
            Phase::Up if drag.cancelled() => self.tray_ui.cancel_drag(),
            // A press that never moved is a click: it opens the editor.
            Phase::Up if !self.tray_ui.is_dragging() => self.edit_tile(id),
            Phase::Up => {
                let dropped = self.tray_ui.dropped(&ids);
                if let (Some((id, to)), Some((key, _))) = (dropped, self.composer.shown()) {
                    let effects = self.composer.move_tile(key, id, to);
                    self.run_composer(effects, Some(tx));
                }
            }
        }
    }

    /// The tile menu (§14.2): secondary click on tile `id` beside the
    /// pointer, or the menu key on the tray beside the current tile.
    pub(crate) fn tray_menu(
        &mut self,
        id: Option<TileId>,
        press: Option<PointerPress>,
        _tx: &mut Tx,
    ) {
        let ids = self.tray_ids();
        let Some(tile) = id.or_else(|| self.tray_ui.current_in(&ids)) else {
            return;
        };
        let from_keyboard = id.is_none();
        let placement = match press {
            Some(press) if !from_keyboard => Placement::AtPointer([press.x, press.y]),
            _ => Placement::Beside(format!("tray-tile-{}", tile.0)),
        };
        // The keyboard comes back to the tray; a pointer leaves it in the
        // field, where writing goes on.
        let trigger = if from_keyboard {
            TRAY_KEY
        } else {
            COMPOSER_KEY
        };
        let items = vec![
            MenuItem::plain(
                "menu-tray-edit",
                strings::tray_edit(),
                MenuAction::EditTile(tile),
                false,
            )
            .with_icon(&icons::CROP),
            MenuItem::plain(
                "menu-tray-remove",
                strings::tray_remove_menu(),
                MenuAction::RemoveTile(tile),
                false,
            )
            .with_icon(&icons::X)
            .with_shortcut(strings::shortcut_delete()),
        ];
        self.nav.close_accounts();
        self.nav.menu = Some(OpenMenu::new(placement, trigger.to_owned(), items));
    }
}
