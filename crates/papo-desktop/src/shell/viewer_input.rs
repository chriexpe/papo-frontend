//! The viewer's input (`docs/design.md` §12, §14.8): its keys, ‹ ›, the zoom buttons, the
//! wheel, and a drag or double click on the room. What they do to the picture is `Viewer`'s;
//! fetching and closing are `shell::viewer`'s.

use rune::{Drag, Key, KeyEvent, Mods, Phase, Tx};

use super::Shell;

impl Shell {
    /// Keys on the viewer (§12, §14.8). Returns whether it took the key.
    pub(crate) fn viewer_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let natural = self.viewer_natural();
        let room = self.viewer_room;
        // Enter or Space on a hidden item reveals it, as a press does.
        let reveals = self.viewer_hidden()
            && key.mods == Mods::default()
            && (key.key == Key::Enter || key.key == Key::Text(" ".to_owned()));
        if reveals {
            self.viewer_reveal();
            return true;
        }
        if key.mods == Mods::default()
            && let Key::Text(typed) = &key.key
            && self.viewer_video_key(typed, tx)
        {
            return true;
        }
        let Some(viewer) = self.viewer.as_mut() else {
            return false;
        };
        let mods = key.mods;
        let plain = mods == Mods::default()
            || mods
                == Mods {
                    shift: true,
                    ..Mods::default()
                };
        let command = key.is_command('=') || key.is_command('+');
        match &key.key {
            Key::Left if plain => return self.viewer_step_tx(-1, tx),
            Key::Right if plain => return self.viewer_step_tx(1, tx),
            Key::Text(text) if (plain && (text == "+" || text == "=")) || command => {
                viewer.zoom_step(true, natural, room);
            }
            Key::Text(text) if (plain && text == "-") || key.is_command('-') => {
                viewer.zoom_step(false, natural, room);
            }
            Key::Text(text) if (plain && text == "0") || key.is_command('0') => viewer.fit(),
            Key::Text(text) if plain && text.eq_ignore_ascii_case("f") => {
                viewer.full_screen = !viewer.full_screen;
                tx.toggle_full_screen();
            }
            Key::Function(11) if mods == Mods::default() => {
                viewer.full_screen = !viewer.full_screen;
                tx.toggle_full_screen();
            }
            _ if key.is_command('c') => self.viewer_copy(tx),
            _ if key.is_command('s') => self.viewer_save(tx),
            Key::Escape => {
                if viewer.full_screen {
                    viewer.full_screen = false;
                    tx.toggle_full_screen();
                } else {
                    self.close_viewer(tx);
                }
            }
            _ => return false,
        }
        true
    }

    /// ‹ or › (or ← →): the previous or next item, its whole picture fetched. Returns whether
    /// it moved.
    pub(crate) fn viewer_step_tx(&mut self, by: isize, tx: &mut Tx) -> bool {
        let moved = self.viewer.as_mut().is_some_and(|viewer| viewer.step(by));
        if moved {
            self.load_viewer_picture(tx);
            self.follow_viewer_video(tx);
        }
        moved
    }

    /// The zoom value's press: back to fit.
    pub(crate) fn viewer_fit(&mut self) {
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.fit();
        }
    }

    /// Zoom in or out a step, toward the centre.
    pub(crate) fn viewer_zoom_step(&mut self, inward: bool) {
        let (natural, room) = (self.viewer_natural(), self.viewer_room);
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.zoom_step(inward, natural, room);
        }
    }

    /// The wheel over the room at `at` (room points from its top left).
    pub(crate) fn viewer_wheel(&mut self, dy: f32, precise: bool, at: [f32; 2]) -> bool {
        let (natural, room) = (self.viewer_natural(), self.viewer_room);
        let toward = [at[0] - room[0] / 2.0, at[1] - room[1] / 2.0];
        if let Some(viewer) = self.viewer.as_mut() {
            viewer.wheel(dy, precise, toward, natural, room);
            return true;
        }
        false
    }

    /// A press or drag on the room (points from its top left): a drag pans a zoomed picture,
    /// a double click fits it or shows 100 % at the pointer.
    pub(crate) fn viewer_drag(&mut self, drag: &mut Drag) {
        let (natural, room) = (self.viewer_natural(), self.viewer_room);
        let Some(viewer) = self.viewer.as_mut() else {
            return;
        };
        let toward = [drag.x - room[0] / 2.0, drag.y - room[1] / 2.0];
        match drag.phase {
            Phase::Down if drag.clicks == 2 => viewer.double_click(toward, natural, room),
            Phase::Down => viewer.drag([drag.x, drag.y], true, natural, room),
            Phase::Move => viewer.drag([drag.x, drag.y], false, natural, room),
            Phase::Up => viewer.drag_end(),
        }
    }
}
