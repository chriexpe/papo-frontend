//! Keys that work from anywhere in the window (`docs/design.md` §12, Go
//! anywhere and Window): Ctrl+1–9 shows the server in that place of the
//! rail; Ctrl+Tab, Ctrl+⇧Tab, Ctrl+PgDn and Ctrl+PgUp move between servers
//! as tabs; Alt+⇧↑ and Alt+⇧↓ move to the previous or next unread channel;
//! Alt+← and Alt+→ go back and forward through this server's channels;
//! Ctrl+W closes the window, Ctrl+Q quits, F11 toggles full screen; Ctrl+/,
//! F1 or Ctrl+? opens the keyboard shortcuts sheet (`shortcuts_sheet`); Ctrl+K
//! opens the jump field (`jump`), from any focus, typing included; Ctrl+=,
//! Ctrl+- and Ctrl+0 make the text larger, smaller or the default size.
//!
//! Owns which key does what. Where the moves go is `navigation`'s; a popup
//! or an open overlay keeps these keys for itself.

use rune::{Key, KeyEvent, Mods, Tx};

use super::Shell;
use super::appearance::TextSizeStep;
use crate::quit_ask::Leaving;

/// Ctrl+W, Ctrl+Q, F11: the window's own keys, from anywhere. Closing
/// and quitting ask first while an upload runs (`quit_ask`).
fn window_key(shell: &mut Shell, key: &KeyEvent, tx: &mut Tx) -> bool {
    let mods = key.mods;
    if key.is_command('w') && primary_only(mods) {
        shell.leave_or_ask(Leaving::Close, tx);
    } else if key.is_command('q') && primary_only(mods) {
        shell.leave_or_ask(Leaving::Quit, tx);
    } else if key.key == Key::Function(11) && mods == Mods::default() {
        tx.toggle_full_screen();
    } else {
        return false;
    }
    true
}

/// Ctrl+/, F1 or Ctrl+?: the keyboard shortcuts sheet.
fn is_shortcuts_key(key: &KeyEvent) -> bool {
    let mods = key.mods;
    let primary = (mods.cmd || mods.ctrl) && !mods.alt && !mods.logo;
    (primary && (key.is_command('/') || key.is_command('?')))
        || (key.key == Key::Function(1) && mods == Mods::default())
}

/// Ctrl+= (or Ctrl++, the same key shifted), Ctrl+- and Ctrl+0: the Text
/// size setting a step larger, smaller, or back to the default.
fn text_size_key(key: &KeyEvent) -> Option<TextSizeStep> {
    let mods = key.mods;
    let primary = (mods.cmd || mods.ctrl) && !mods.alt && !mods.logo;
    if !primary {
        return None;
    }
    if key.is_command('=') || key.is_command('+') {
        Some(TextSizeStep::Larger)
    } else if key.is_command('-') && !mods.shift {
        Some(TextSizeStep::Smaller)
    } else if key.is_command('0') && !mods.shift {
        Some(TextSizeStep::Default)
    } else {
        None
    }
}

/// Ctrl (⌘ on macOS) alone.
fn primary_only(mods: Mods) -> bool {
    (mods.cmd || mods.ctrl) && !mods.alt && !mods.shift && !mods.logo
}

impl Shell {
    /// One of this module's keys, if it is one. Returns whether it was
    /// taken.
    pub(super) fn app_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        if window_key(self, key, tx) {
            return true;
        }
        // The sheet and Ctrl+K draw over the running app only, and never
        // take the keyboard from a composition in progress.
        let running = matches!(&self.stage, super::Stage::Running { flow: None, .. });
        let composing = self
            .composer
            .shown()
            .is_some_and(|(_, draft)| draft.edit.is_composing());
        if !running || composing {
            return false;
        }
        // The viewer keeps the keyboard: the app's keys wait until it closes.
        if self.viewer.is_some() {
            return false;
        }
        // The picker holds the keyboard: its tabs' keys switch it or close it, the app's wait.
        if self.picker.is_some() {
            if key.is_command('e') && primary_only(key.mods) {
                self.emoji_button_pressed(tx);
                return true;
            }
            if key.is_command('g') && primary_only(key.mods) {
                self.gif_button_pressed(tx);
                return true;
            }
            return false;
        }
        if is_shortcuts_key(key) {
            self.toggle_shortcuts(tx);
            return true;
        }
        if key.is_command('k') && primary_only(key.mods) {
            self.toggle_jump(tx);
            return true;
        }
        if key.is_command(',') && primary_only(key.mods) {
            self.toggle_settings(tx);
            return true;
        }
        if let Some(step) = text_size_key(key) {
            self.step_text_size(step, tx);
            return true;
        }
        // A popup or an overlay holds the keyboard like a dialog does.
        if self.popup_shown() || self.nav.accounts_open() || self.nav.direct_accounts_open() {
            return false;
        }
        // Ctrl+O: attach files, the system's key (§12); not from behind the
        // settings panel.
        if key.is_command('o') && primary_only(key.mods) && self.settings.is_none() {
            self.attach_pressed(tx);
            return true;
        }
        // Ctrl+E: the Emoji tab, for the shown field (§16.5); not from behind the settings
        // panel.
        if key.is_command('e') && primary_only(key.mods) && self.settings.is_none() {
            self.emoji_button_pressed(tx);
            return true;
        }
        // Ctrl+G: the GIF tab, likewise.
        if key.is_command('g') && primary_only(key.mods) && self.settings.is_none() {
            self.gif_button_pressed(tx);
            return true;
        }
        self.server_number_key(key, tx)
            || self.server_step_key(key, tx)
            || self.unread_step_key(key, tx)
            || self.history_key(key, tx)
    }

    /// The shortcuts sheet opens over everything, or closes.
    pub(crate) fn toggle_shortcuts(&mut self, tx: &mut Tx) {
        if self.shortcuts_open {
            self.close_shortcuts();
            return;
        }
        // Only over the app itself: a dialog or a menu keeps its keys.
        if self.popup_shown() {
            return;
        }
        self.shortcuts_open = true;
        tx.focus(crate::shortcuts_sheet::SHORTCUTS_ROWS_KEY, true);
    }

    /// Escape, or the sheet's key again.
    pub(crate) fn close_shortcuts(&mut self) {
        self.shortcuts_open = false;
    }

    /// Ctrl+1–9: the server in that place of the rail.
    fn server_number_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        if !primary_only(key.mods) {
            return false;
        }
        let Some(place) = ('1'..='9').position(|digit| key.is_command(digit)) else {
            return false;
        };
        if let Some(server) = self
            .nav
            .servers()
            .servers
            .get(place)
            .map(|server| server.id)
        {
            self.select_server(server, tx);
        }
        true
    }

    /// Ctrl+Tab / Ctrl+PgDn to the next server, Ctrl+⇧Tab / Ctrl+PgUp to
    /// the previous one, round the ends, as tabs do.
    fn server_step_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        let ctrl = (mods.cmd || mods.ctrl) && !mods.alt && !mods.logo;
        let by: isize = match key.key {
            Key::Tab | Key::PageDown if ctrl && !mods.shift => 1,
            Key::Tab if ctrl => -1,
            Key::PageUp if ctrl && !mods.shift => -1,
            _ => return false,
        };
        let servers = &self.nav.servers().servers;
        if servers.is_empty() {
            return true;
        }
        let count = servers.len();
        let from = self
            .nav
            .selected()
            .and_then(|selected| servers.iter().position(|server| server.id == selected.id));
        let to = match from {
            // From Direct, the first or last server.
            None if by > 0 => 0,
            None => count - 1,
            Some(from) if by > 0 => (from + 1) % count,
            Some(from) => (from + count - 1) % count,
        };
        let server = servers[to].id;
        self.select_server(server, tx);
        true
    }

    /// Alt+⇧↑ / Alt+⇧↓: the previous or next unread channel, round the
    /// ends; nothing moves when no other is unread.
    pub(super) fn unread_step_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        let alt_shift = mods.alt && mods.shift && !mods.cmd && !mods.ctrl && !mods.logo;
        let by = match key.key {
            Key::Up if alt_shift => -1,
            Key::Down if alt_shift => 1,
            _ => return false,
        };
        if self.nav.direct_shown() {
            return false;
        }
        if let Some(next) = self.nav.unread_channel_beside(by).map(str::to_owned) {
            self.select_channel(&next, tx);
        }
        true
    }

    /// Alt+← / Alt+→: back and forward through this server's channels.
    pub(super) fn history_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        let alt_alone = mods.alt && !mods.shift && !mods.cmd && !mods.ctrl && !mods.logo;
        let forward = match key.key {
            Key::Left if alt_alone => false,
            Key::Right if alt_alone => true,
            _ => return false,
        };
        if self.nav.direct_shown() {
            return false;
        }
        self.worked_outside();
        if self.nav.step_history(forward).is_some() {
            self.sync_timeline(Some(tx));
        }
        true
    }
}
