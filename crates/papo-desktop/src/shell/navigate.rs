//! The shell's side of navigation: taking the server list the SDK posts,
//! selecting a server or channel (also by Alt+↑/↓), moving a server in the
//! rail, and the server menu with its remove confirmation.
//!
//! The transitions are `Navigation`'s; this sends what they produce to the
//! SDK and keeps the decoded server icons.

use papo_sdk::ServerId;
use rune::{Drag, Key, KeyEvent, Phase, Tx};

use super::Shell;
use crate::channel_pane::{SERVER_CARD_KEY, SERVER_OPTIONS_KEY, channel_key};
use crate::composer::COMPOSER_KEY;
use crate::menu::{MenuAction, MenuItem, MessageAction, OpenMenu, PersonAction, Placement};
use crate::rail::tile_key;
use crate::remove_server;
use crate::server_list::ServerList;
use crate::strings;

impl Shell {
    /// Takes the SDK's newest server list, and decodes the icons that
    /// changed.
    pub(crate) fn servers_changed(&mut self, list: ServerList, tx: &mut Tx) -> bool {
        let shown = |list: &ServerList| -> Vec<_> {
            list.servers
                .iter()
                .filter_map(|server| server.account.as_ref().map(|account| account.id))
                .collect()
        };
        // Ctrl+K's places belong to the accounts shown when it opened.
        if shown(self.nav.servers()) != shown(&list) {
            self.jump_accounts_changed();
        }
        if let Some(images) = &self.images {
            self.icons
                .retain(|id, _| list.get(*id).is_some_and(|server| server.icon.is_some()));
            for server in &list.servers {
                let Some(bytes) = &server.icon else {
                    continue;
                };
                let fresh = self
                    .icon_bytes
                    .get(&server.id)
                    .is_none_or(|seen| !std::sync::Arc::ptr_eq(seen, bytes));
                if fresh {
                    self.icons.insert(server.id, images.load(bytes.to_vec()));
                    self.icon_bytes
                        .insert(server.id, std::sync::Arc::clone(bytes));
                }
            }
            self.icon_bytes.retain(|id, _| self.icons.contains_key(id));
        }
        self.nav.servers_changed(list);
        // An account switch: the other account's conversation goes.
        self.drop_unshown_dms();
        self.retain_timelines();
        self.sync_timeline(Some(tx));
        true
    }

    pub(crate) fn select_server(&mut self, server: ServerId, tx: &mut Tx) {
        self.worked_outside();
        // A card's Message no longer takes the person back to Direct.
        self.awaited_dm = None;
        self.nav.select_server(server);
        self.sync_timeline(Some(tx));
    }

    pub(crate) fn select_channel(&mut self, channel: &str, tx: &mut Tx) {
        self.worked_outside();
        self.nav.select_channel(channel);
        self.sync_timeline(Some(tx));
    }

    /// Keys no control took: Alt+↑/↓ between channels, then Ctrl+A in the
    /// timeline.
    pub(crate) fn on_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        self.app_key(key, tx)
            || self.channel_step_key(key, tx)
            || self.select_message_key(key, tx)
            || self.pins_key(key, tx)
            || self.search_shortcut(key, tx)
            || self.profile_key(key, tx)
            || self.direct_key(key, tx)
            || self.escape_key(key)
    }

    /// Escape where nothing took it (the rail, a channel row, the header):
    /// what is on top closes.
    fn escape_key(&mut self, key: &KeyEvent) -> bool {
        let mods = key.mods;
        let bare = !(mods.cmd || mods.ctrl || mods.alt || mods.shift || mods.logo);
        key.key == Key::Escape && bare && self.escape_on_top()
    }

    /// Ctrl+P: the pins panel opens, stretches back, or closes.
    fn pins_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        if !key.is_command('p') || mods.shift || mods.alt || self.popup_shown() {
            return false;
        }
        self.pins_pressed(true, tx);
        true
    }

    /// Ctrl+F: the search panel opens, or stretches back, with the
    /// keyboard in its field.
    fn search_shortcut(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        if !key.is_command('f') || mods.shift || mods.alt || self.popup_shown() {
            return false;
        }
        self.search_pressed(true, tx);
        true
    }

    /// Ctrl+⇧D (`docs/design.md` §12): Direct, as its rail tile does.
    fn direct_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        if !key.is_command('d') || !mods.shift || mods.alt || self.popup_shown() {
            return false;
        }
        self.select_direct(tx);
        true
    }

    /// Ctrl+U in a DM: its profile pane (in a channel, the members pane's
    /// key, which comes with the roles session).
    fn profile_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        if !key.is_command('u') || mods.shift || mods.alt || self.popup_shown() {
            return false;
        }
        if self.nav.picked_dm().is_none() {
            return false;
        }
        self.toggle_dm_profile(tx);
        true
    }

    /// Alt+↑ or Alt+↓ (Alt alone: Alt+⇧ is the unread channels'; macOS's
    /// ⌥⌘↑/↓ come with the macOS port, D12): the
    /// previous or next channel, stopping at the list's ends. Focus decides
    /// (`docs/design.md` §3): a focused rail tile takes the keys itself and
    /// moves, so they reach here from anywhere else; the composer asks
    /// before its editor would take them. A channel row that has the
    /// keyboard hands it to the newly selected row; the field keeps it,
    /// also across a voice channel, which has none (`Composer::show`).
    pub(super) fn channel_step_key(&mut self, key: &KeyEvent, tx: &mut Tx) -> bool {
        let mods = key.mods;
        let alt_alone = mods.alt && !mods.shift && !mods.cmd && !mods.ctrl && !mods.logo;
        let by = match key.key {
            Key::Up if alt_alone => -1,
            Key::Down if alt_alone => 1,
            _ => return false,
        };
        // The accounts grown from ⇄ hold the keyboard like a popup does.
        if self.popup_shown() || self.nav.accounts_open() || self.nav.selected_channel().is_none() {
            return false;
        }
        let Some(next) = self.nav.channel_beside(by).map(str::to_owned) else {
            // At an end: the key is spent, nothing moves.
            return true;
        };
        let row_focused = match (tx.focused(), self.nav.selected_channel()) {
            (Some(focused), Some(current)) => focused == channel_key(current),
            _ => false,
        };
        self.select_channel(&next, tx);
        if row_focused {
            // From the keyboard, so the ring shows on the row it lands on.
            tx.focus(channel_key(&next), true);
        }
        true
    }

    /// Alt+↑ or Alt+↓ on a focused tile.
    pub(crate) fn nudge_server(&mut self, server: ServerId, by: isize) {
        if let Some(order) = self.nav.nudge(server, by) {
            self.send_order(order);
        }
    }

    /// A drag on a tile: the rail previews where it lands, and the drop sends
    /// the new order.
    pub(crate) fn drag_server(&mut self, server: ServerId, drag: &Drag, tx: &mut Tx) {
        match drag.phase {
            Phase::Down => {}
            Phase::Move => {
                self.nav.drag_moved(server, drag.dy);
                // The tiles the dragged one passes step aside on their springs.
                tx.animated();
            }
            Phase::Up if drag.cancelled() => self.nav.drag_cancelled(),
            Phase::Up => {
                if let Some(order) = self.nav.drag_ended() {
                    self.send_order(order);
                }
            }
        }
    }

    /// Sends `order` to the SDK, one at a time: while one is on its way, the
    /// newest order waits and goes when it answers, so saves land in the
    /// order they were made.
    fn send_order(&mut self, order: Vec<ServerId>) {
        if self.reorder.in_flight {
            self.reorder.waiting = Some(order);
            return;
        }
        let Some(sdk) = self.sdk.clone() else {
            self.nav.reorder_failed();
            return;
        };
        self.reorder.in_flight = true;
        let ticket = self.reorder_requests.issue();
        sdk.request(
            ticket,
            |shell: &Shell| &shell.reorder_requests,
            move |client| async move {
                client
                    .reorder_servers(order)
                    .await
                    .inspect_err(|error| log::warn!("the servers were not reordered: {error}"))
                    .is_ok()
            },
            Shell::reorder_answered,
        );
    }

    fn reorder_answered(&mut self, saved: bool) -> bool {
        self.reorder.in_flight = false;
        match self.reorder.waiting.take() {
            Some(next) => {
                self.send_order(next);
                false
            }
            None if !saved => {
                self.nav.reorder_failed();
                true
            }
            None => false,
        }
    }

    /// The server card's ⚙: the menu grows from the card. ⚙ takes the
    /// keyboard: pressed on the grown pill, the accounts closing would
    /// otherwise hand it to ⇄, and the menu would give it back there.
    pub(crate) fn open_server_menu(&mut self, tx: &mut Tx) {
        if let Some(server) = self.nav.selected().map(|server| server.id) {
            let trigger = SERVER_OPTIONS_KEY.to_owned();
            self.open_menu(Placement::GrowFrom(SERVER_CARD_KEY), trigger, server);
            tx.focus(SERVER_OPTIONS_KEY, false);
        }
    }

    /// The menu key or a secondary click on a tile: the menu beside it.
    pub(crate) fn open_server_menu_beside(&mut self, server: ServerId) {
        self.open_menu(
            Placement::Beside(tile_key(server)),
            tile_key(server),
            server,
        );
    }

    fn open_menu(&mut self, placement: Placement, trigger: String, server: ServerId) {
        // ⚙ shows on the grown pill too; the menu takes the pill's place.
        self.nav.close_accounts();
        let items = vec![MenuItem::plain(
            "menu-remove-server",
            strings::remove_server(),
            MenuAction::RemoveServer(server),
            true,
        )];
        self.nav.menu = Some(OpenMenu::new(placement, trigger, items));
    }

    /// The open menu, to change in place (the strip's chosen emoji).
    pub(crate) fn open_menu_mut(&mut self) -> Option<&mut OpenMenu> {
        self.nav.menu.as_mut()
    }

    pub(crate) fn close_menu(&mut self) {
        self.nav.menu = None;
    }

    /// Closes the menu, gives the keyboard back to its trigger, so a popup
    /// the action opens returns there, and carries the action out.
    pub(crate) fn menu_picked(&mut self, action: MenuAction, tx: &mut Tx) {
        // View reactions keeps the menu: it turns into the list.
        if let MenuAction::Message {
            key,
            id,
            action: MessageAction::ViewReactions,
        } = action
        {
            self.show_reactions(key, id, tx);
            return;
        }
        // So does View profile: it turns into the author's card.
        if let MenuAction::Message {
            key,
            id,
            action: MessageAction::ViewProfile,
        } = action
        {
            self.show_author_card(key, id, tx);
            return;
        }
        // Asking about a ban, or not, keeps the card.
        if let MenuAction::Person {
            action: action @ (PersonAction::AskBan | PersonAction::CancelBan),
            ..
        } = action
        {
            self.card_ban_asked(action == PersonAction::AskBan, tx);
            return;
        }
        if let Some(menu) = self.nav.menu.take() {
            tx.focus(menu.trigger, false);
        }
        match action {
            MenuAction::RemoveServer(server) => self.nav.removing = Some(server),
            MenuAction::RemoveTile(tile) => {
                self.remove_tile(tile, tx);
            }
            MenuAction::RetrySend {
                key,
                local,
                unknown,
            } => {
                self.retry_send(key, local, unknown);
                if !unknown {
                    // The mark becomes the waiting clock, which takes no
                    // keyboard: back to writing.
                    tx.focus(COMPOSER_KEY, false);
                }
            }
            MenuAction::DiscardSend { key, local } => {
                self.discard_send(key, local);
                // The row goes with it.
                tx.focus(COMPOSER_KEY, false);
            }
            MenuAction::RetryChange { key, local } => {
                self.retry_change(key, local);
                // The mark becomes the waiting clock: back to writing.
                tx.focus(COMPOSER_KEY, false);
            }
            MenuAction::DiscardChange { key, local } => {
                self.discard_change(key, local);
                tx.focus(COMPOSER_KEY, false);
            }
            MenuAction::Message { key, id, action } => self.message_action(key, id, action, tx),
            MenuAction::OpenLink { key, url } => self.link_pressed(key, url, tx),
            MenuAction::CopyLink(url) => tx.copy(url),
            MenuAction::React { key, id, emoji } => {
                self.toggle_reaction(key, id, &emoji);
            }
            MenuAction::Person { key, user, action } => self.person_action(key, user, action, tx),
            MenuAction::Setting(choice) => self.settings_chosen(choice, tx),
            MenuAction::SearchScope { channel } => {
                let scope = if channel {
                    super::search::Scope::Channel
                } else {
                    super::search::Scope::Server
                };
                self.search_scope_picked(scope, tx);
            }
        }
    }

    pub(crate) fn cancel_remove_server(&mut self) {
        self.nav.removing = None;
    }

    /// Removes the server, and gives the keyboard to the server card's ⚙,
    /// which stays while the next server is selected.
    pub(crate) fn confirm_remove_server(&mut self, tx: &mut Tx) {
        if let Some(server) = self.nav.removing.take() {
            remove_server::remove(self, server);
            tx.focus(SERVER_OPTIONS_KEY, false);
        }
    }

    /// Open on an already-added server with a working account.
    pub(crate) fn open_server(&mut self, server: ServerId) {
        self.end_sign_in();
        self.nav.select_server(server);
        self.sync_timeline(None);
    }
}
