//! The Notifications page (`docs/design.md` §8, Notifications): account-scoped
//! switches backed by the server (`whoami` / `PUT /users/settings`), the
//! device-only Tray counter switch, and a link row to the active server's card
//! for per-channel notification modes.
//!
//! Owns the page's rows and nav summary; what each change saves is the shell's
//! (`shell/settings_notifications`).

use super::{Context, Group, Kind, Row};
use crate::settings::RowId;
use crate::strings;

pub(super) fn notifications(cx: &Context<'_>) -> Vec<Group> {
    let mut groups = Vec::new();
    if let Some(account) = cx.account
        && let Some(policy) = account.notification_policy
    {
        let mut rows = vec![Row {
            id: RowId::NotifyMessages,
            title: strings::settings_notify_messages().to_owned(),
            sub: Some(strings::settings_notify_messages_sub().to_owned()),
            kind: Kind::Switch(policy.enabled),
        }];
        // Left out while Message notifications is off — never greyed (§8).
        if policy.enabled {
            rows.push(Row {
                id: RowId::NotifyMentions,
                title: strings::settings_notify_mentions().to_owned(),
                sub: None,
                kind: Kind::Switch(policy.mentions),
            });
            rows.push(Row {
                id: RowId::NotifyPreview,
                title: strings::settings_notify_preview().to_owned(),
                sub: Some(strings::settings_notify_preview_sub().to_owned()),
                kind: Kind::Switch(policy.message_preview),
            });
            rows.push(Row {
                id: RowId::NotifySound,
                title: strings::settings_notify_sound().to_owned(),
                sub: None,
                kind: Kind::Switch(policy.sound),
            });
        }
        rows.push(Row {
            id: RowId::NotifySyncNote,
            title: strings::settings_notify_sync_note(&account.server),
            sub: None,
            kind: Kind::Note,
        });
        groups.push(Group { title: None, rows });
    }
    groups.push(Group {
        title: None,
        rows: vec![Row {
            id: RowId::TrayCounter,
            title: strings::settings_tray_counter().to_owned(),
            sub: Some(strings::settings_tray_counter_sub().to_owned()),
            kind: Kind::Switch(cx.notifications.tray_counter),
        }],
    });
    if cx.account.is_some() {
        groups.push(Group {
            title: None,
            rows: vec![Row {
                id: RowId::ChannelNotifications,
                title: strings::settings_channel_notifications().to_owned(),
                sub: Some(strings::settings_channel_notifications_sub().to_owned()),
                kind: Kind::ChevronAction,
            }],
        });
    }
    groups
}

/// The Notifications nav row's one-line summary: "On · tray counter", "On", or
/// "Off".
pub(crate) fn notifications_summary(cx: &Context<'_>) -> &'static str {
    let enabled = cx
        .account
        .and_then(|account| account.notification_policy)
        .is_none_or(|policy| policy.enabled);
    strings::settings_notifications_summary(enabled, cx.notifications.tray_counter)
}
