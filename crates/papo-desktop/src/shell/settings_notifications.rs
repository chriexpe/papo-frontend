//! The shell's side of Settings › Notifications (`docs/design.md` §8): account
//! notification switches sent through `Operations::change_notification_settings`
//! (one call at a time, confirmed values only), the device-only Tray counter
//! switch saved to `device_settings`, and Channel notifications opening the
//! active server's card.

use papo_sdk::operations::settings::NotificationPolicyChange;
use rune::Tx;

use super::Shell;
use crate::notification_settings::NotificationPrefs;
use crate::settings::{Change, RowId, Target};

impl Shell {
    /// Applies a device notification preference change (`Tray counter`) and
    /// writes it off the UI thread.
    pub(crate) fn set_notification_prefs(
        &mut self,
        prefs: NotificationPrefs,
        row: Option<RowId>,
        tx: &mut Tx,
    ) {
        self.latest_rows.notifications = row;
        if let Some(next) = self.notification_prefs.change(prefs) {
            self.write_notification_prefs(next, row, tx);
        }
    }

    fn write_notification_prefs(
        &mut self,
        prefs: NotificationPrefs,
        row: Option<RowId>,
        tx: &mut Tx,
    ) {
        let write = self.notification_prefs.write_of(prefs);
        self.run_write(
            write,
            move |shell, ok, tx| shell.notification_prefs_written(prefs, row, ok, tx),
            tx,
        );
    }

    fn notification_prefs_written(
        &mut self,
        prefs: NotificationPrefs,
        row: Option<RowId>,
        ok: bool,
        tx: &mut Tx,
    ) {
        if !ok {
            log::warn!("a notification device setting was not saved");
        }
        let next = self.notification_prefs.written(prefs, ok);
        if let (Some(panel), Some(row)) = (self.settings.as_mut(), row) {
            panel.saved(row, Target::Notifications(prefs), ok);
        }
        if let Some(next) = next {
            self.write_notification_prefs(next, self.latest_rows.notifications, tx);
        }
    }

    /// Sends one account notification setting toggle through the SDK's
    /// serialized `whoami` + `PUT /users/settings` operation.
    pub(crate) fn save_notification_policy(
        &mut self,
        row: RowId,
        change: NotificationPolicyChange,
    ) {
        let Some(account) = self.subject_account() else {
            return;
        };
        self.account_call(
            row,
            Target::NotificationPolicy(change),
            move |client| async move {
                let session = client.session(account)?;
                session
                    .operations()
                    .change_notification_settings(change)
                    .await?;
                Ok(())
            },
        );
    }

    /// Closes Settings and opens the active server's card for per-channel
    /// notification modes.
    pub(crate) fn open_channel_notifications(&mut self, tx: &mut Tx) {
        let Some((server, _)) = self.settings_subject() else {
            return;
        };
        let server_id = server.id;
        self.close_settings();
        self.nav.select_server(server_id);
        self.sync_timeline(Some(tx));
        self.open_server_card(tx);
    }
}

/// Returns the account notification change when `change` is one of the
/// account-scoped switches on Settings › Notifications.
pub(super) fn account_change(change: &Change) -> Option<(RowId, NotificationPolicyChange)> {
    let Change::Switch(row, on) = *change else {
        return None;
    };
    let mut policy = NotificationPolicyChange::default();
    match row {
        RowId::NotifyMessages => policy.enabled = Some(on),
        RowId::NotifyMentions => policy.mentions = Some(on),
        RowId::NotifyPreview => policy.message_preview = Some(on),
        RowId::NotifySound => policy.sound = Some(on),
        _ => return None,
    }
    Some((row, policy))
}

/// Returns the next device notification preferences when `change` is a
/// device-scoped switch on Settings › Notifications.
pub(super) fn device_applied(change: &Change) -> Option<NotificationPrefs> {
    let Change::Switch(RowId::TrayCounter, on) = *change else {
        return None;
    };
    Some(NotificationPrefs { tray_counter: on })
}
