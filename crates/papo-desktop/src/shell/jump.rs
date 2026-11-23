//! Ctrl+K, jump anywhere (`docs/design.md` §12), from the shell's side: it
//! opens a field over the app from any focus, typing included. With nothing
//! typed it lists the last three jumps, then the conversations with
//! mentions, then the other unread ones, then the ones shown most recently
//! (Christian, 2026-11-23); typing asks the SDK for one ranked list across
//! every server the rail shows (`jump_targets`, with its `#` `@` `*`
//! prefixes). ↑ ↓ and Enter or a click jump; Escape closes it.
//!
//! A jump shows a channel, a conversation or a server, or opens the
//! conversation with a person, then gives the keyboard to the text field.
//! Owns the field's state and what a jump does; the look is `jump_view`'s.

use std::collections::BTreeSet;

use papo_sdk::{AccountId, Uuid};
use papo_sdk_ui::{JumpKind, JumpTarget, jump_targets};
use rune::{KeyEvent, TextEdit, Tx};

use super::Shell;
use crate::badges::Badge;
use crate::composer::COMPOSER_KEY;
use crate::recent_jumps::RECENT_JUMPS;

/// The element key of the jump field.
pub(crate) const JUMP_FIELD_KEY: &str = "jump-field";
/// How many places an empty field lists.
const EMPTY_LIST_ROWS: usize = 12;

/// The element key of a target's row.
pub(crate) fn target_key(target: &JumpTarget) -> String {
    let id = match target.kind {
        JumpKind::Channel { channel } | JumpKind::Direct { channel, .. } => channel.to_string(),
        JumpKind::Person { user } => user.to_string(),
        JumpKind::Server => "server".to_owned(),
    };
    format!(
        "jump-{}-{}-{id}",
        target.account.server(),
        target.account.user()
    )
}

/// The open jump field.
pub(crate) struct Jump {
    pub(crate) edit: TextEdit,
    /// What the field finds, in order.
    pub(crate) found: Vec<JumpTarget>,
    /// The highlighted row: what Enter jumps to.
    pub(crate) highlight: usize,
    /// Each ask takes the next; only the newest answer counts.
    asked: u64,
}

impl Shell {
    /// Ctrl+K: the field opens with the keyboard in it, or closes.
    pub(crate) fn toggle_jump(&mut self, tx: &mut Tx) {
        if self.jump.take().is_some() {
            return;
        }
        if self.popup_shown() {
            return;
        }
        self.nav.dismiss_overlays();
        self.jump = Some(Jump {
            edit: TextEdit::new(""),
            found: self.places_to_start(),
            highlight: 0,
            asked: 0,
        });
        tx.focus(JUMP_FIELD_KEY, true);
    }

    /// Escape: the field closes.
    pub(crate) fn close_jump(&mut self) {
        self.jump = None;
    }

    /// The field Rune edits; without one, a spare nothing reads.
    pub(crate) fn jump_field_mut(&mut self) -> &mut TextEdit {
        let Shell {
            jump, search_spare, ..
        } = self;
        jump.as_mut().map_or_else(
            || {
                *search_spare = TextEdit::new("");
                search_spare
            },
            |jump| &mut jump.edit,
        )
    }

    /// The field changed: an empty one lists the places to start from, else
    /// the SDK is asked.
    pub(crate) fn jump_edited(&mut self, _tx: &mut Tx) {
        let start = self.places_to_start();
        let accounts = self.shown_accounts();
        let Some(jump) = self.jump.as_mut() else {
            return;
        };
        jump.highlight = 0;
        jump.asked += 1;
        if jump.edit.text.trim().is_empty() {
            jump.found = start;
            return;
        }
        let (asked, query) = (jump.asked, jump.edit.text.clone());
        let Some(sdk) = &self.sdk else {
            return;
        };
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let snapshots: Vec<_> = accounts
                .iter()
                .filter_map(|account| {
                    let session = client.session(*account).ok()?;
                    let snapshot = session.subscribe_resources().borrow().clone();
                    Some((*account, snapshot))
                })
                .collect();
            let borrowed: Vec<_> = snapshots
                .iter()
                .map(|(account, snapshot)| (*account, snapshot.as_ref()))
                .collect();
            let found = jump_targets(&borrowed, &query);
            updates.send(move |shell| shell.jump_found(asked, found));
        });
    }

    /// Ask `asked`'s answer, if it is still the newest.
    pub(super) fn jump_found(&mut self, asked: u64, found: Vec<JumpTarget>) -> bool {
        let Some(jump) = self.jump.as_mut().filter(|jump| jump.asked == asked) else {
            return false;
        };
        jump.found = found;
        jump.highlight = 0;
        true
    }

    /// ↑ ↓ through the places, Enter to jump, Escape to close.
    pub(crate) fn jump_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        if mods.ctrl || mods.alt || mods.cmd || mods.logo {
            return false;
        }
        let Some(jump) = self.jump.as_mut() else {
            return false;
        };
        let rows = jump.found.len();
        match key.key {
            rune::Key::Down | rune::Key::Up if rows > 0 => {
                let by = if key.key == rune::Key::Down { 1 } else { -1 };
                jump.highlight = crate::composer::picker::wrapped_step(jump.highlight, rows, by);
                if let Some(target) = jump.found.get(jump.highlight) {
                    tx.scroll_into_view(target_key(target));
                }
                true
            }
            rune::Key::Enter if rows > 0 => {
                let at = jump.highlight;
                self.jump_picked(at, tx);
                true
            }
            rune::Key::Escape => {
                self.close_jump();
                true
            }
            _ => false,
        }
    }

    /// Row `at` was picked: the field closes and the jump happens.
    pub(crate) fn jump_picked(&mut self, at: usize, tx: &mut Tx) {
        let Some(target) = self
            .jump
            .take()
            .and_then(|jump| jump.found.get(at).cloned())
        else {
            return;
        };
        self.recent_jumps.jumped(&target);
        match target.kind {
            JumpKind::Channel { channel } => {
                self.select_server(target.account.server(), tx);
                self.select_channel(&channel.to_string(), tx);
            }
            JumpKind::Direct { channel, peer } => {
                let row = self
                    .direct
                    .iter()
                    .find(|row| row.id.account == target.account && row.id.channel == channel)
                    .cloned();
                self.select_direct(tx);
                match row {
                    Some(row) => self.pick_dm(row, tx),
                    // Closed since: messaging them reopens it.
                    None => self.open_dm(target.account, peer),
                }
            }
            JumpKind::Person { user } => {
                self.select_direct(tx);
                self.open_dm(target.account, user);
            }
            JumpKind::Server => self.select_server(target.account.server(), tx),
        }
        tx.focus(COMPOSER_KEY, true);
    }

    /// Each server's shown account, in the rail's order.
    fn shown_accounts(&self) -> Vec<AccountId> {
        self.nav
            .servers()
            .servers
            .iter()
            .filter_map(|server| server.account.as_ref().map(|account| account.id))
            .collect()
    }

    /// What an empty field lists: the last jumps, then conversations with
    /// mentions, then other unread ones, then the most recently shown, each
    /// once.
    fn places_to_start(&self) -> Vec<JumpTarget> {
        let accounts = self.shown_accounts();
        let mut places = self.recent_jumps.targets(&accounts);
        places.truncate(RECENT_JUMPS);
        let mut unread: Vec<(Badge, JumpTarget)> = self.unread_places();
        // Mentions first, then the rest, keeping the rail's order.
        unread.sort_by_key(|(badge, _)| badge.mentions().is_none());
        places.extend(unread.into_iter().map(|(_, target)| target));
        places.extend(self.recent_places());
        let mut seen = BTreeSet::new();
        places.retain(|target| seen.insert(target_key(target)));
        places.truncate(EMPTY_LIST_ROWS);
        places
    }

    /// Every unread channel and conversation, in the rail's order.
    fn unread_places(&self) -> Vec<(Badge, JumpTarget)> {
        let mut unread = Vec::new();
        for server in &self.nav.servers().servers {
            let Some(account) = server.account.as_ref().map(|account| account.id) else {
                continue;
            };
            for channel in &server.text_channels {
                let Ok(id) = Uuid::parse_str(&channel.id) else {
                    continue;
                };
                if channel.badge.is_unread() {
                    let target = place(account, JumpKind::Channel { channel: id }, &channel.name);
                    unread.push((channel.badge, target));
                }
            }
            for row in self.direct.iter().filter(|row| row.id.account == account) {
                let badge = server
                    .direct_badges
                    .get(&row.id.channel)
                    .copied()
                    .unwrap_or_default();
                if badge.is_unread() {
                    unread.push((badge, dm_place(row)));
                }
            }
        }
        unread
    }

    /// The conversations shown most recently, with their names.
    fn recent_places(&self) -> Vec<JumpTarget> {
        self.tabs
            .recently_shown()
            .filter_map(|key| {
                if let Some(row) = self
                    .direct
                    .iter()
                    .find(|row| row.id.account == key.account && row.id.channel == key.channel)
                {
                    return Some(dm_place(row));
                }
                let server =
                    self.nav.servers().servers.iter().find(|server| {
                        server.account.as_ref().is_some_and(|a| a.id == key.account)
                    })?;
                let channel = server
                    .text_channels
                    .iter()
                    .find(|channel| channel.id == key.channel.to_string())?;
                let kind = JumpKind::Channel {
                    channel: key.channel,
                };
                Some(place(key.account, kind, &channel.name))
            })
            .collect()
    }
}

/// A conversation as a place to jump to.
fn dm_place(row: &papo_sdk_ui::DirectMessageItem) -> JumpTarget {
    let name = papo_sdk_ui::display_name(row.nickname.as_deref(), row.username.as_deref())
        .unwrap_or(crate::strings::someone());
    JumpTarget {
        account: row.id.account,
        kind: JumpKind::Direct {
            channel: row.id.channel,
            peer: row.user,
        },
        name: name.to_owned(),
        username: row.username.clone(),
    }
}

fn place(account: AccountId, kind: JumpKind, name: &str) -> JumpTarget {
    JumpTarget {
        account,
        kind,
        name: name.to_owned(),
        username: None,
    }
}
