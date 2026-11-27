//! Where the person is in the app: Direct or the selected server and, per
//! server, the selected channel; the direct conversation picked in Direct;
//! the rail's order while a reorder is on its way to the SDK or a tile is
//! being dragged; the open menu, the open accounts and the remove
//! confirmation.
//!
//! Owns these transitions and nothing the SDK holds: the server list itself
//! comes from `server_list`, and reordering or removing is sent to the SDK by
//! the shell. Selection is per window and not saved yet (D2 keeps it).

use std::collections::HashMap;
use std::sync::Arc;

use papo_sdk::{AccountId, ServerId};
use papo_sdk_ui::DirectMessageItem;

use crate::design::{RAIL_GAP_PT, RAIL_TILE_PT};
use crate::menu::OpenMenu;
use crate::server_list::{ServerEntry, ServerList};

/// The distance from one rail tile to the next, in points.
pub(crate) const RAIL_STEP_PT: f32 = RAIL_TILE_PT + RAIL_GAP_PT;

/// The app's navigation state.
#[derive(Default)]
pub(crate) struct Navigation {
    servers: ServerList,
    selected: Option<ServerId>,
    /// Direct is shown in the selected server's place; the server stays
    /// remembered for when the person goes back to it.
    direct: bool,
    /// Direct shows the people the accounts block instead of the
    /// conversations.
    direct_blocked: bool,
    /// The direct conversation picked in Direct, as its row last read: it
    /// stays picked when it leaves the list (a block, a hide elsewhere), so
    /// the conversation can say why.
    picked_dm: Option<Arc<DirectMessageItem>>,
    /// The channel picked on each server, by the server's channel id.
    channels: HashMap<ServerId, String>,
    /// Each server's history of shown channels, for Back and Forward.
    history: HashMap<ServerId, moves::ChannelHistory>,
    /// The order sent to the SDK, shown until the registry has it.
    pending_order: Option<Vec<ServerId>>,
    drag: Option<RailDrag>,
    pub(crate) menu: Option<OpenMenu>,
    /// The server whose accounts are open: its pill grown from ⇄.
    accounts: Option<ServerId>,
    /// The server whose card is open: its pill grown downward.
    server_card: Option<ServerId>,
    /// Whether Direct's accounts are open: every account on every server.
    direct_accounts: bool,
    /// The server whose removal waits for the person to confirm.
    pub(crate) removing: Option<ServerId>,
}

/// A rail tile being dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RailDrag {
    pub(crate) server: ServerId,
    /// Its place when the drag started.
    from: usize,
    /// Where it would land now.
    to: usize,
    /// The pointer's travel since the press, in points.
    dy_pt: f32,
}

impl Navigation {
    pub(crate) fn servers(&self) -> &ServerList {
        &self.servers
    }

    /// Takes the SDK's newest server list. A server that went away takes its
    /// selection with it: the first server is selected instead.
    pub(crate) fn servers_changed(&mut self, list: ServerList) {
        if self.pending_order.as_deref() == Some(list.order().as_slice()) {
            self.pending_order = None;
        }
        let before = self.servers.clone();
        self.servers = list;
        let order = self.servers.order();
        self.channels.retain(|server, _| order.contains(server));
        // A server keeps its history while it shows the same account: one
        // removed, or showing another account, starts anew.
        let shown = |list: &ServerList, server: ServerId| {
            list.get(server)
                .and_then(|entry| entry.account.as_ref().map(|account| account.id))
        };
        let servers = &self.servers;
        self.history.retain(|server, _| {
            servers.get(*server).is_some() && shown(&before, *server) == shown(servers, *server)
        });
        if self
            .selected
            .is_none_or(|selected| !order.contains(&selected))
        {
            self.selected = order.first().copied();
        }
        if let Some(removing) = self.removing
            && !order.contains(&removing)
        {
            self.removing = None;
        }
        if self.accounts.is_some_and(|server| !order.contains(&server)) {
            self.accounts = None;
        }
        if self
            .server_card
            .is_some_and(|server| !order.contains(&server))
        {
            self.server_card = None;
        }
    }

    /// The rail in the order to draw: a drag's preview, else the order on its
    /// way to the SDK, else the saved order.
    pub(crate) fn shown(&self) -> Vec<&ServerEntry> {
        let order = self.order_shown();
        order
            .iter()
            .filter_map(|id| self.servers.get(*id))
            .collect()
    }

    fn order_shown(&self) -> Vec<ServerId> {
        let mut order = self.order();
        if let Some(drag) = self.drag {
            moved(&mut order, drag.from, drag.to);
        }
        order
    }

    /// The order as last sent or saved, without a drag in progress.
    fn order(&self) -> Vec<ServerId> {
        let saved = self.servers.order();
        match &self.pending_order {
            // Only while it still names exactly the saved servers.
            Some(pending) if same_servers(pending, &saved) => pending.clone(),
            _ => saved,
        }
    }

    /// The selected server; none while Direct is shown.
    pub(crate) fn selected(&self) -> Option<&ServerEntry> {
        if self.direct {
            return None;
        }
        self.selected.and_then(|id| self.servers.get(id))
    }

    /// Whether Direct is shown.
    pub(crate) fn direct_shown(&self) -> bool {
        self.direct
    }

    /// Whether Direct shows the blocked people instead of the
    /// conversations.
    pub(crate) fn direct_blocked(&self) -> bool {
        self.direct_blocked
    }

    /// Switches Direct between the conversations and the blocked people.
    pub(crate) fn toggle_direct_blocked(&mut self) {
        self.direct_blocked = !self.direct_blocked;
    }

    /// Shows Direct in the server's place; the server's accounts and card close.
    pub(crate) fn select_direct(&mut self) {
        self.direct = true;
        self.accounts = None;
        self.server_card = None;
    }

    /// The direct conversation picked in Direct, while Direct is shown.
    pub(crate) fn picked_dm(&self) -> Option<&DirectMessageItem> {
        self.picked_dm.as_deref().filter(|_| self.direct)
    }

    /// Picks direct conversation `row`, showing Direct.
    pub(crate) fn pick_dm(&mut self, row: Arc<DirectMessageItem>) {
        self.select_direct();
        self.picked_dm = Some(row);
    }

    /// The picked conversation's row, Direct shown or not.
    pub(crate) fn picked_dm_row(&self) -> Option<&DirectMessageItem> {
        self.picked_dm.as_deref()
    }

    /// Picks no direct conversation.
    pub(crate) fn unpick_dm(&mut self) {
        self.picked_dm = None;
    }

    /// Takes the newest list: the picked row follows its update, and stays
    /// as it was when it left the list.
    pub(crate) fn direct_changed<'a>(
        &mut self,
        rows: impl IntoIterator<Item = &'a Arc<DirectMessageItem>>,
    ) {
        let Some(picked) = &self.picked_dm else {
            return;
        };
        if let Some(row) = rows.into_iter().find(|row| row.id == picked.id) {
            self.picked_dm = Some(Arc::clone(row));
        }
    }

    /// Forgets the picked conversation when its account signed out.
    pub(crate) fn account_closed(&mut self, account: AccountId) {
        if self
            .picked_dm
            .as_ref()
            .is_some_and(|picked| picked.id.account == account)
        {
            self.picked_dm = None;
        }
    }

    /// Selects server `id`, leaving Direct; the accounts of another server
    /// close.
    pub(crate) fn select_server(&mut self, id: ServerId) {
        if self.servers.get(id).is_some() {
            self.direct = false;
            self.selected = Some(id);
            if self.accounts != Some(id) {
                self.accounts = None;
            }
            if self.server_card != Some(id) {
                self.server_card = None;
            }
        }
    }

    /// Whether the selected server's card is open.
    pub(crate) fn server_card_open(&self) -> bool {
        !self.direct && self.server_card.is_some() && self.server_card == self.selected
    }

    /// Opens the selected server's card, closing the other overlays.
    pub(crate) fn open_server_card(&mut self) {
        self.dismiss_overlays();
        self.server_card = self.selected;
    }

    /// Closes the server card.
    pub(crate) fn close_server_card(&mut self) {
        self.server_card = None;
    }

    /// Whether the selected server's accounts are open.
    pub(crate) fn accounts_open(&self) -> bool {
        self.accounts.is_some() && self.accounts == self.selected
    }

    /// Opens the selected server's accounts, closing the other overlays.
    pub(crate) fn open_accounts(&mut self) {
        self.dismiss_overlays();
        self.accounts = self.selected;
    }

    pub(crate) fn close_accounts(&mut self) {
        self.accounts = None;
        self.direct_accounts = false;
    }

    /// Whether Direct's accounts are open.
    pub(crate) fn direct_accounts_open(&self) -> bool {
        self.direct_accounts && self.direct_shown()
    }

    /// Opens Direct's accounts, closing the other overlays.
    pub(crate) fn open_direct_accounts(&mut self) {
        self.dismiss_overlays();
        self.direct_accounts = true;
    }

    /// The selected server's channel: the one picked, else its first text
    /// channel.
    pub(crate) fn selected_channel(&self) -> Option<&str> {
        let server = self.selected()?;
        let picked = self.channels.get(&server.id).filter(|picked| {
            server
                .text_channels
                .iter()
                .chain(&server.voice_channels)
                .any(|channel| &channel.id == *picked)
        });
        picked.map(String::as_str).or_else(|| {
            server
                .text_channels
                .first()
                .map(|channel| channel.id.as_str())
        })
    }

    /// The selected server's channel `by` places after the selected one
    /// (`-1` the one before), in the list's order: text channels, then
    /// voice. `None` past either end.
    pub(crate) fn channel_beside(&self, by: isize) -> Option<&str> {
        let server = self.selected()?;
        let selected = self.selected_channel()?;
        let mut channels = server.text_channels.iter().chain(&server.voice_channels);
        let from = channels
            .clone()
            .position(|channel| channel.id == selected)?;
        let to = from.checked_add_signed(by)?;
        channels.nth(to).map(|channel| channel.id.as_str())
    }

    /// Shows `channel` in the selected server; the one left is where Back
    /// goes.
    pub(crate) fn select_channel(&mut self, channel: &str) {
        let left = self.selected_channel().map(str::to_owned);
        if let Some(server) = self.selected {
            if let Some(left) = left.filter(|left| left != channel) {
                self.history.entry(server).or_default().left(left);
            }
            self.channels.insert(server, channel.to_owned());
        }
    }

    /// Moves `server` one place up (`-1`) or down (`1`). Returns the order to
    /// send to the SDK, or `None` when it cannot move that way.
    pub(crate) fn nudge(&mut self, server: ServerId, by: isize) -> Option<Vec<ServerId>> {
        let order = self.order();
        let from = order.iter().position(|id| *id == server)?;
        let to = from.checked_add_signed(by).filter(|to| *to < order.len())?;
        self.commit_move(order, from, to)
    }

    /// A drag on `server`'s tile moved by `dy_pt` since the press.
    pub(crate) fn drag_moved(&mut self, server: ServerId, dy_pt: f32) {
        let order = self.order();
        let Some(from) = order.iter().position(|id| *id == server) else {
            return;
        };
        let to = target(from, dy_pt, order.len());
        self.drag = Some(RailDrag {
            server,
            from,
            to,
            dy_pt,
        });
    }

    /// The drag ended. Returns the order to send when the tile moved. Its
    /// place is found again by server, in case the list changed meanwhile.
    pub(crate) fn drag_ended(&mut self) -> Option<Vec<ServerId>> {
        let drag = self.drag.take()?;
        let order = self.order();
        let from = order.iter().position(|id| *id == drag.server)?;
        let to = target(from, drag.dy_pt, order.len());
        self.commit_move(order, from, to)
    }

    /// The drag was cancelled (Escape, or a modal took the pointer): the tile
    /// goes back and nothing is sent.
    pub(crate) fn drag_cancelled(&mut self) {
        self.drag = None;
    }

    /// Clears what a newly opened screen covers: a drag, the menu and the
    /// remove confirmation.
    pub(crate) fn dismiss_overlays(&mut self) {
        self.drag = None;
        self.menu = None;
        self.accounts = None;
        self.server_card = None;
        self.direct_accounts = false;
        self.removing = None;
    }

    /// How far the dragged tile is drawn from its place in the preview, in
    /// points, so it stays under the pointer.
    pub(crate) fn drag_offset(&self, server: ServerId) -> f32 {
        match self.drag {
            Some(drag) if drag.server == server => {
                #[expect(
                    clippy::cast_precision_loss,
                    reason = "a rail holds far fewer than 2^23 servers"
                )]
                let moved_pt = (drag.to as f32 - drag.from as f32) * RAIL_STEP_PT;
                drag.dy_pt - moved_pt
            }
            _ => 0.0,
        }
    }

    /// The SDK refused the order sent: show the saved one again.
    pub(crate) fn reorder_failed(&mut self) {
        self.pending_order = None;
    }

    fn commit_move(
        &mut self,
        mut order: Vec<ServerId>,
        from: usize,
        to: usize,
    ) -> Option<Vec<ServerId>> {
        if from == to {
            return None;
        }
        moved(&mut order, from, to);
        self.pending_order = Some(order.clone());
        Some(order)
    }
}

/// Where a tile dragged `dy_pt` from place `from` lands, among `count`.
fn target(from: usize, dy_pt: f32, count: usize) -> usize {
    let steps = (dy_pt / RAIL_STEP_PT).round();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a rail holds far fewer servers than an isize counts"
    )]
    let steps = steps as isize;
    from.saturating_add_signed(steps)
        .min(count.saturating_sub(1))
}

fn moved(order: &mut Vec<ServerId>, from: usize, to: usize) {
    if from < order.len() && to < order.len() {
        let server = order.remove(from);
        order.insert(to, server);
    }
}

fn same_servers(a: &[ServerId], b: &[ServerId]) -> bool {
    a.len() == b.len() && a.iter().all(|id| b.contains(id))
}

mod moves;

#[cfg(test)]
mod tests;
