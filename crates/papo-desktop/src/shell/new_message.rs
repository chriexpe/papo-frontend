//! The New message picker from the shell's side (`docs/design.md` §10,
//! Direct messages): New message in Direct's pill opens it as a popover
//! grown down from the pill, with the keyboard in its "Find someone"
//! field; the people are asked for as the field changes, from every
//! server the rail shows; ↑ ↓ and Enter (or a click) pick one, which
//! opens or reopens the conversation with them (`open_dm`).
//!
//! Owns the picker's state. The people and their order are the SDK's
//! (`direct_candidates`); the overlay (Escape, a click outside, focus going
//! back) is the menu's; the look is `direct_pane::new_message`'s.

use papo_sdk::AccountId;
use papo_sdk_ui::{DirectCandidate, direct_candidates};
use rune::{KeyEvent, TextEdit, Tx};

use super::Shell;
use crate::menu::{OpenMenu, Placement};

/// The element key of Direct's New message button.
pub(crate) const NEW_MESSAGE_BUTTON_KEY: &str = "direct-new-message";
/// The element key of Direct's pill, which the picker grows from.
pub(crate) const DIRECT_PILL_KEY: &str = "direct-pill";
/// The element key of the picker's field.
pub(crate) const FIND_FIELD_KEY: &str = "direct-find";
/// The picker's width when the pill is not laid out yet, in points.
const PICKER_WIDTH_PT: f32 = 260.0;

/// The element key of the picker's row for `candidate`.
pub(crate) fn candidate_key(candidate: &DirectCandidate) -> String {
    format!("find-{}-{}", candidate.account.user(), candidate.user)
}

/// The open picker.
pub(crate) struct DmPicker {
    pub(crate) edit: TextEdit,
    /// The people the field's text finds, in the SDK's order.
    pub(crate) found: Vec<DirectCandidate>,
    /// The highlighted row: what Enter picks.
    pub(crate) highlight: usize,
    /// Each ask takes the next; only the newest answer counts.
    asked: u64,
}

impl Shell {
    /// New message: the picker opens with the keyboard in its field.
    pub(crate) fn new_message_pressed(&mut self, tx: &mut Tx) {
        let mut menu = OpenMenu::with_entries(
            Placement::GrowFrom(DIRECT_PILL_KEY),
            NEW_MESSAGE_BUTTON_KEY.to_owned(),
            Vec::new(),
            PICKER_WIDTH_PT,
        );
        menu.people = true;
        self.nav.menu = Some(menu);
        self.dm_picker = Some(DmPicker {
            edit: TextEdit::new(""),
            found: Vec::new(),
            highlight: 0,
            asked: 0,
        });
        self.find_people();
        tx.focus(FIND_FIELD_KEY, true);
    }

    /// The picker, while the menu shows it.
    pub(crate) fn shown_dm_picker(&self) -> Option<&DmPicker> {
        self.nav
            .menu
            .as_ref()
            .filter(|menu| menu.people)
            .and(self.dm_picker.as_ref())
    }

    /// The field Rune edits; without a picker, a spare nothing reads.
    pub(crate) fn find_field_mut(&mut self) -> &mut TextEdit {
        let Shell {
            dm_picker,
            search_spare,
            ..
        } = self;
        dm_picker.as_mut().map_or_else(
            || {
                *search_spare = TextEdit::new("");
                search_spare
            },
            |picker| &mut picker.edit,
        )
    }

    /// The field changed: the people it finds are asked for.
    pub(crate) fn find_edited(&mut self, _tx: &mut Tx) {
        if let Some(picker) = self.dm_picker.as_mut() {
            picker.highlight = 0;
        }
        self.find_people();
    }

    /// Asks the SDK for the people the field finds on every server the
    /// rail shows, in its order.
    fn find_people(&mut self) {
        let accounts: Vec<AccountId> = self
            .nav
            .servers()
            .servers
            .iter()
            .filter_map(|server| server.account.as_ref().map(|account| account.id))
            .collect();
        let Some(picker) = self.dm_picker.as_mut() else {
            return;
        };
        picker.asked += 1;
        let (asked, query) = (picker.asked, picker.edit.text.clone());
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
            let found = direct_candidates(&borrowed, &query);
            updates.send(move |shell| shell.people_found(asked, found));
        });
    }

    /// The people ask `asked` found, if it is still the newest.
    pub(super) fn people_found(&mut self, asked: u64, found: Vec<DirectCandidate>) -> bool {
        let Some(picker) = self
            .dm_picker
            .as_mut()
            .filter(|picker| picker.asked == asked)
        else {
            return false;
        };
        picker.found = found;
        picker.highlight = picker.highlight.min(picker.found.len().saturating_sub(1));
        true
    }

    /// ↑ ↓ through the people, Enter to pick, Escape to close. Returns
    /// whether the key was taken.
    pub(crate) fn find_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        if mods.ctrl || mods.alt || mods.cmd || mods.logo {
            return false;
        }
        let Some(picker) = self.dm_picker.as_mut() else {
            return false;
        };
        let rows = picker.found.len();
        match key.key {
            rune::Key::Down | rune::Key::Up if rows > 0 => {
                let by = if key.key == rune::Key::Down { 1 } else { -1 };
                picker.highlight =
                    crate::composer::picker::wrapped_step(picker.highlight, rows, by);
                if let Some(candidate) = picker.found.get(picker.highlight) {
                    tx.scroll_into_view(candidate_key(candidate));
                }
                true
            }
            rune::Key::Enter if rows > 0 => {
                let at = picker.highlight;
                self.person_picked(at, tx);
                true
            }
            rune::Key::Escape => {
                self.close_dm_picker(tx);
                true
            }
            _ => false,
        }
    }

    /// Row `at` was picked: the picker closes and the conversation with
    /// that person, from that row's account, opens.
    pub(crate) fn person_picked(&mut self, at: usize, tx: &mut Tx) {
        let Some(candidate) = self
            .dm_picker
            .as_ref()
            .and_then(|picker| picker.found.get(at).cloned())
        else {
            return;
        };
        self.close_dm_picker(tx);
        self.open_dm(candidate.account, candidate.user);
    }

    /// The picker closes; New message gets the keyboard back.
    pub(crate) fn close_dm_picker(&mut self, tx: &mut Tx) {
        if self.nav.menu.as_ref().is_some_and(|menu| menu.people) {
            self.nav.menu = None;
        }
        self.dm_picker = None;
        tx.focus(NEW_MESSAGE_BUTTON_KEY, true);
    }
}
