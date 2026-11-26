//! The shell's side of OS notifications (`docs/design.md` §8): tracking window
//! focus and visible channel for SDK notification suppression, clearing posted
//! OS notifications when a channel is shown, and routing a clicked OS
//! notification to its account, server/Direct, channel, and message.

use std::collections::BTreeSet;

use papo_sdk::{AccountId, Uuid};
use rune::Tx;

use super::Shell;
use crate::notifications::ClickTarget;
use crate::timeline::TimelineKey;
use crate::timeline::rows::message_key;

/// Shell runtime state for OS notification suppression and click routing.
pub(super) struct NotificationsRuntime {
    focused: bool,
    visible_channel: Option<(AccountId, Uuid)>,
    pending_click: Option<ClickTarget>,
}

impl Default for NotificationsRuntime {
    fn default() -> Self {
        Self {
            focused: true,
            visible_channel: None,
            pending_click: None,
        }
    }
}

impl Shell {
    /// Window focus changed (`App::on_focus`): updates visible-channel
    /// suppression so notifications are suppressed only while the window is
    /// both shown and focused on that channel.
    pub(crate) fn window_focus(&mut self, focused: bool, _tx: &mut Tx) {
        self.notifications.focused = focused;
        self.sync_visible_channel();
    }

    /// Tells the SDK which channel (if any) is currently visible and focused
    /// on screen, and clears posted OS notifications for that channel.
    pub(crate) fn sync_visible_channel(&mut self) {
        let visible = (self.window_shown
            && self.notifications.focused
            && self.settings.is_none()
            && !self.popup_shown())
        .then(|| self.wanted_timeline())
        .flatten()
        .map(|key| (key.account, key.channel));
        if visible == self.notifications.visible_channel {
            return;
        }
        self.notifications.visible_channel = visible;
        if let Some((account, channel)) = visible {
            crate::notifications::clear_channel(account, channel);
        }
        let Some(sdk) = &self.sdk else {
            return;
        };
        sdk.run(move |client| async move {
            let registry = client.subscribe_registry().borrow().clone();
            for saved in &registry.accounts {
                let Ok(session) = client.session(saved.id) else {
                    continue;
                };
                let channels = match visible {
                    Some((account, channel)) if account == saved.id => BTreeSet::from([channel]),
                    _ => BTreeSet::new(),
                };
                if let Err(error) = session.notifications().set_visible_channels(channels) {
                    log::debug!("visible notification channels were not updated: {error}");
                }
            }
        });
    }

    /// An OS notification was clicked: switches account, server/Direct, and
    /// channel as needed, and jumps to and flashes the target message.
    pub(crate) fn notification_clicked(&mut self, target: ClickTarget, tx: &mut Tx) {
        self.close_settings();
        self.nav.dismiss_overlays();
        self.shortcuts_open = false;
        self.viewer = None;
        self.pins_panel = None;
        self.jump = None;
        self.notifications.pending_click = Some(target);
        self.try_open_pending_notification(tx);
    }

    /// Advances a pending notification click once its account, DM list, or
    /// timeline has settled.
    pub(super) fn try_open_pending_notification(&mut self, tx: &mut Tx) {
        let Some(target) = self.notifications.pending_click.clone() else {
            return;
        };
        let Some(server) = self
            .nav
            .servers()
            .servers
            .iter()
            .find(|server| server.id.to_string() == target.server)
            .cloned()
        else {
            self.notifications.pending_click = None;
            return;
        };
        let account = AccountId::new(server.id, target.user);
        if !server.accounts.iter().any(|saved| saved.id == account) {
            self.notifications.pending_click = None;
            return;
        }
        if server.account.as_ref().map(|shown| shown.id) != Some(account) {
            if let Some(sdk) = &self.sdk {
                let server_id = server.id;
                sdk.run(move |client| async move {
                    if let Err(error) = client.set_active_account(server_id, Some(account)).await {
                        log::warn!("notification click could not switch account: {error}");
                    }
                });
            }
            return;
        }

        crate::notifications::clear_channel(account, target.channel);

        let key = TimelineKey {
            account,
            channel: target.channel,
        };
        if target.is_dm {
            let row = self
                .direct
                .iter()
                .find(|row| row.id.account == account && row.id.channel == target.channel)
                .cloned();
            if let Some(row) = row {
                let already = self
                    .nav
                    .picked_dm()
                    .is_some_and(|picked| picked.id == row.id);
                if !already {
                    self.pick_dm(row, tx);
                }
            } else {
                if !self.nav.direct_shown() {
                    self.select_direct(tx);
                }
                return;
            }
        } else {
            let channel_str = target.channel.to_string();
            if !server
                .text_channels
                .iter()
                .any(|channel| channel.id == channel_str)
            {
                self.notifications.pending_click = None;
                return;
            }
            let already = self.nav.selected().is_some_and(|sel| sel.id == server.id)
                && self.nav.selected_channel() == Some(channel_str.as_str());
            if !already {
                self.nav.select_server(server.id);
                self.nav.select_channel(&channel_str);
                self.sync_timeline(Some(tx));
            }
        }

        let Some(open) = self.shown_timeline(key) else {
            return;
        };
        if open.holds(target.message) {
            self.notifications.pending_click = None;
            self.jump_to_original(key, target.message, tx);
            tx.focus(message_key(target.message), false);
        } else if !open.items.is_empty() {
            // The timeline finished loading its initial window without holding
            // this older message; leave the channel open and stop waiting.
            self.notifications.pending_click = None;
        }
    }
}
