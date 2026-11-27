//! The shell's side of the server card (`docs/design.md` §8, "D9 additions"):
//! toggling the grown card from the server pill, copying the server address,
//! opening and applying per-channel (and All channels) notification mode menus,
//! running Mark all as read, and removing the server from the card.

use std::time::Duration;

use papo_sdk::operations::settings::UpdateChannelUserSettingRequest;
use papo_sdk::{ServerId, Uuid};
use rune::Tx;

use super::Shell;
use crate::menu::{MENU_KEY, MenuAction, MenuItem, OpenMenu, Placement};
use crate::server_card::{
    CardState, ChannelNotifyMode, SERVER_CARD_ALL_CHANNELS_KEY, SERVER_CARD_COPY_KEY,
    SERVER_CARD_OPEN_KEY, channel_notify_key, common_mode,
};

/// How long "Copied" shows after copying the server address, in seconds.
const COPIED_FOR_S: f64 = 2.0;

/// Transient state for the open server card.
#[derive(Default)]
pub(super) struct ServerCardRuntime {
    copied_until_s: Option<f64>,
    mode_failed: bool,
    mark_all_failed: bool,
}

impl Shell {
    /// Current transient state shown inside the server card.
    pub(crate) fn server_card_state(&self) -> CardState {
        let online = self
            .nav
            .selected()
            .and_then(|server| server.account.as_ref())
            .and_then(|account| self.people(account.id))
            .filter(|activity| activity.live)
            .map_or(0, |activity| activity.presence.len());
        CardState {
            online,
            copied: self.server_card.copied_until_s.is_some(),
            mode_failed: self.server_card.mode_failed,
            mark_all_failed: self.server_card.mark_all_failed,
        }
    }

    /// Toggles the selected server's card from its pill body.
    pub(crate) fn toggle_server_card(&mut self, tx: &mut Tx) {
        if self.nav.server_card_open() {
            self.close_server_card();
            tx.focus(SERVER_CARD_OPEN_KEY, false);
            return;
        }
        self.open_server_card(tx);
    }

    /// Opens the selected server's card.
    pub(crate) fn open_server_card(&mut self, tx: &mut Tx) {
        if self.nav.selected().is_none() {
            return;
        }
        self.server_card = ServerCardRuntime::default();
        self.nav.open_server_card();
        tx.focus(SERVER_CARD_COPY_KEY, false);
    }

    /// Closes the server card (Escape or click outside).
    pub(crate) fn close_server_card(&mut self) {
        self.nav.close_server_card();
        self.server_card = ServerCardRuntime::default();
    }

    /// Copies the selected server's address to the clipboard.
    pub(crate) fn copy_server_address(&mut self, tx: &mut Tx) {
        let Some(server) = self.nav.selected() else {
            return;
        };
        tx.copy(server.address.clone());
        self.server_card.copied_until_s = Some(tx.clock() + COPIED_FOR_S);
        tx.wake_after(Duration::from_secs_f64(COPIED_FOR_S));
    }

    /// Clears the "Copied" state once its timer expires.
    pub(super) fn poll_server_card(&mut self, clock_s: f64) -> bool {
        if self
            .server_card
            .copied_until_s
            .is_some_and(|until| until <= clock_s)
        {
            self.server_card.copied_until_s = None;
            return true;
        }
        false
    }

    /// Opens the three-mode notification menu for one channel or All channels.
    pub(crate) fn open_channel_notify_menu(
        &mut self,
        server_id: ServerId,
        channel: Option<&str>,
        tx: &mut Tx,
    ) {
        let Some(server) = self.nav.servers().get(server_id) else {
            return;
        };
        let (trigger, target_channel, current) = match channel {
            Some(id) => {
                let Some(entry) = server.text_channels.iter().find(|ch| ch.id == id) else {
                    return;
                };
                let Ok(uuid) = Uuid::parse_str(id) else {
                    return;
                };
                (
                    channel_notify_key(id),
                    Some(uuid),
                    ChannelNotifyMode::from_backend(entry.notification_mode.as_ref()),
                )
            }
            None => (
                SERVER_CARD_ALL_CHANNELS_KEY.to_owned(),
                None,
                common_mode(&server.text_channels),
            ),
        };
        let items: Vec<MenuItem> = ChannelNotifyMode::ALL
            .into_iter()
            .map(|mode| {
                MenuItem::plain(
                    &format!("channel-notify-mode-{mode:?}"),
                    mode.label(),
                    MenuAction::ChannelNotify {
                        server: server_id,
                        channel: target_channel,
                        mode,
                    },
                    false,
                )
                .with_radio(current == Some(mode))
            })
            .collect();
        self.nav.menu = Some(OpenMenu::new(
            Placement::BelowEnd(trigger.clone()),
            trigger,
            items,
        ));
        tx.focus(MENU_KEY, true);
    }

    /// Saves a chosen channel notification mode (for one channel or every text
    /// channel on `server_id`).
    pub(crate) fn apply_channel_notify(
        &mut self,
        server_id: ServerId,
        channel: Option<Uuid>,
        mode: ChannelNotifyMode,
    ) {
        let (Some(sdk), Some(server)) = (self.sdk.clone(), self.nav.servers().get(server_id))
        else {
            return;
        };
        let Some(account) = server.account.as_ref().map(|account| account.id) else {
            return;
        };
        let channels: Vec<Uuid> = match channel {
            Some(id) => vec![id],
            None => server
                .text_channels
                .iter()
                .filter_map(|entry| Uuid::parse_str(&entry.id).ok())
                .collect(),
        };
        if channels.is_empty() {
            return;
        }
        self.server_card.mode_failed = false;
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let Ok(session) = client.session(account) else {
                updates.send(|shell: &mut Shell| {
                    shell.server_card.mode_failed = true;
                    true
                });
                return;
            };
            let mut failed = false;
            for ch in channels {
                let request = UpdateChannelUserSettingRequest {
                    notification_settings: mode.as_wire().to_owned(),
                };
                if let Err(error) = session
                    .operations()
                    .update_channel_settings(ch, request)
                    .await
                {
                    log::warn!("channel notification setting was not saved: {error}");
                    failed = true;
                }
            }
            if failed {
                updates.send(|shell: &mut Shell| {
                    shell.server_card.mode_failed = true;
                    true
                });
            }
        });
    }

    /// Marks every unread admitted text channel on `server_id` as read through
    /// the SDK's read path, leaving DMs untouched.
    pub(crate) fn mark_all_read(&mut self, server_id: ServerId) {
        let (Some(sdk), Some(server)) = (self.sdk.clone(), self.nav.servers().get(server_id))
        else {
            return;
        };
        let Some(account) = server.account.as_ref().map(|account| account.id) else {
            return;
        };
        self.server_card.mark_all_failed = false;
        let updates = sdk.updates();
        sdk.run(move |client| async move {
            let ok = match client.session(account) {
                Ok(session) => session
                    .mark_all_read()
                    .await
                    .inspect_err(|error| log::warn!("mark all as read failed: {error}"))
                    .is_ok(),
                Err(error) => {
                    log::warn!("mark all as read had no session: {error}");
                    false
                }
            };
            if ok {
                crate::notifications::clear_account(account);
            } else {
                updates.send(|shell: &mut Shell| {
                    shell.server_card.mark_all_failed = true;
                    true
                });
            }
        });
    }

    /// Remove server from the server card: closes the card and opens the
    /// removal confirmation.
    pub(crate) fn ask_remove_server_from_card(&mut self, server_id: ServerId, tx: &mut Tx) {
        self.close_server_card();
        self.nav.removing = Some(server_id);
        tx.focus("remove-cancel", true);
    }
}
