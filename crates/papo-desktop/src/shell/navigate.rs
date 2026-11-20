//! The shell's side of navigation: taking the server list the SDK posts,
//! selecting a server or channel, moving a server in the rail, and the
//! server menu with its remove confirmation.
//!
//! The transitions are `Navigation`'s; this sends what they produce to the
//! SDK and keeps the decoded server icons.

use papo_sdk::ServerId;
use rune::{Drag, Phase, Tx};

use super::Shell;
use crate::channel_pane::{SERVER_CARD_KEY, SERVER_OPTIONS_KEY};
use crate::menu::{MenuAction, MenuItem, OpenMenu, Placement};
use crate::rail::tile_key;
use crate::remove_server;
use crate::server_list::ServerList;
use crate::strings;

impl Shell {
    /// Takes the SDK's newest server list, and decodes the icons that
    /// changed.
    pub(crate) fn servers_changed(&mut self, list: ServerList) -> bool {
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
        true
    }

    pub(crate) fn select_server(&mut self, server: ServerId) {
        self.nav.select_server(server);
    }

    pub(crate) fn select_channel(&mut self, channel: &str) {
        self.nav.select_channel(channel);
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
        let items = vec![MenuItem {
            key: "menu-remove-server",
            label: strings::remove_server(),
            action: MenuAction::RemoveServer(server),
            danger: true,
        }];
        self.nav.menu = Some(OpenMenu::new(placement, trigger, items));
    }

    pub(crate) fn close_menu(&mut self) {
        self.nav.menu = None;
    }

    /// Closes the menu, gives the keyboard back to its trigger, so a popup
    /// the action opens returns there, and carries the action out.
    pub(crate) fn menu_picked(&mut self, action: MenuAction, tx: &mut Tx) {
        if let Some(menu) = self.nav.menu.take() {
            tx.focus(menu.trigger, false);
        }
        match action {
            MenuAction::RemoveServer(server) => self.nav.removing = Some(server),
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
    }
}
