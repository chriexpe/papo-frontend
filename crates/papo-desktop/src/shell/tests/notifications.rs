//! Headless tests for Settings › Notifications, the server card, and OS
//! notification click routing.

use papo_sdk::{AccountNotificationPolicy, BackendMode, Generation, Uuid};

use super::navigation::{click, entry, named};
use super::{FRAME_S, Shell, app, press, tree};
use crate::badges::Badge;
use crate::notifications::ClickTarget;
use crate::server_card::{
    SERVER_CARD_ALL_CHANNELS_KEY, SERVER_CARD_COPY_KEY, SERVER_CARD_MARK_ALL_KEY,
    SERVER_CARD_OPEN_KEY, SERVER_CARD_PANEL_KEY, SERVER_CARD_REMOVE_KEY, channel_notify_key,
};
use crate::server_list::{ChannelEntry, ServerList};
use crate::settings::{RowId, SETTINGS_BUTTON_KEY};

fn uuid_id(n: u128) -> String {
    Uuid::from_u128(n).to_string()
}

fn with_notifiable_servers() -> rune::Runtime<Shell> {
    let mut rt = app();
    let mut lumen = entry("Lumen", &[]);
    lumen.member_count = 12;
    lumen.notification_policy = Some(AccountNotificationPolicy {
        generation: Generation::for_test(1),
        enabled: true,
        mentions: true,
        message_preview: true,
        sound: true,
    });
    lumen.text_channels = vec![
        ChannelEntry {
            id: uuid_id(1),
            name: "general".to_owned(),
            topic: None,
            notification_mode: Some(BackendMode::All),
            badge: Badge::default(),
        },
        ChannelEntry {
            id: uuid_id(2),
            name: "media".to_owned(),
            topic: None,
            notification_mode: Some(BackendMode::OnlyMentions),
            badge: Badge::default(),
        },
    ];
    let mut mirante = entry("Mirante", &[]);
    mirante.text_channels = vec![ChannelEntry {
        id: uuid_id(3),
        name: "avisos".to_owned(),
        topic: None,
        notification_mode: Some(BackendMode::Off),
        badge: Badge::default(),
    }];
    rt.update(|shell, tx| {
        shell.servers_changed(
            ServerList {
                servers: vec![lumen, mirante],
                direct: Badge::default(),
                account_badges: std::collections::BTreeMap::new(),
            },
            tx,
        );
    });
    rt.frame(FRAME_S);
    rt
}

#[test]
fn server_card_opens_from_pill_and_shows_modes_and_actions() {
    let mut rt = with_notifiable_servers();
    click(&mut rt, SERVER_CARD_OPEN_KEY);
    assert!(rt.find(SERVER_CARD_PANEL_KEY).is_some());
    let shown = tree(&rt);
    assert!(shown.contains("0 online · 12 people"), "{shown}");
    assert!(shown.contains("Lumen.example.invalid"), "{shown}");
    assert!(shown.contains("All messages"), "{shown}");
    assert!(shown.contains("Mentions only"), "{shown}");
    assert!(shown.contains("All channels"), "{shown}");
    assert!(shown.contains("Mark all as read"), "{shown}");
    assert!(shown.contains("Remove server"), "{shown}");

    click(&mut rt, SERVER_CARD_COPY_KEY);
    assert!(tree(&rt).contains("Copied"));

    // Open per-channel notification menu and verify the three radio choices.
    click(&mut rt, &channel_notify_key(&uuid_id(1)));
    let menu_tree = tree(&rt);
    assert!(menu_tree.contains("All messages"), "{menu_tree}");
    assert!(menu_tree.contains("Mentions only"), "{menu_tree}");
    assert!(menu_tree.contains("Off"), "{menu_tree}");
    assert!(rt.find("menu").is_some());

    // Picking a choice closes the menu and leaves the server card open.
    click(&mut rt, "channel-notify-mode-Mentions");
    assert!(rt.find("menu").is_none());
    assert!(rt.find(SERVER_CARD_PANEL_KEY).is_some());

    // All channels row opens the same three modes.
    click(&mut rt, SERVER_CARD_ALL_CHANNELS_KEY);
    assert!(rt.find("menu").is_some());
    press(&mut rt, rune::Key::Escape);
    assert!(rt.find("menu").is_none());

    // Mark all as read and Remove server are present and reachable.
    assert!(rt.find(SERVER_CARD_MARK_ALL_KEY).is_some());
    click(&mut rt, SERVER_CARD_REMOVE_KEY);
    assert!(rt.find(SERVER_CARD_PANEL_KEY).is_none());
    assert!(rt.find("remove-server").is_some());
}

#[test]
fn settings_notifications_toggles_tray_counter_and_opens_server_card() {
    let mut rt = with_notifiable_servers();
    click(&mut rt, SETTINGS_BUTTON_KEY);
    click(&mut rt, "settings-nav-Notifications");
    let shown = tree(&rt);
    assert!(shown.contains("Message notifications"), "{shown}");
    assert!(shown.contains("Notify mentions"), "{shown}");
    assert!(
        shown.contains("Show the message in the notification"),
        "{shown}"
    );
    assert!(shown.contains("Sound"), "{shown}");
    assert!(
        shown.contains("These follow your account on Lumen."),
        "{shown}"
    );
    assert!(shown.contains("Tray counter"), "{shown}");
    assert!(shown.contains("This device only"), "{shown}");
    assert!(shown.contains("Channel notifications"), "{shown}");

    assert!(rt.state.notification_prefs.settings().tray_counter);
    click(&mut rt, &RowId::TrayCounter.key());
    assert!(!rt.state.notification_prefs.settings().tray_counter);

    // Channel notifications closes Settings and opens the active server's card.
    click(&mut rt, &RowId::ChannelNotifications.key());
    assert!(rt.state.settings().is_none());
    assert!(rt.find(SERVER_CARD_PANEL_KEY).is_some());
}

#[test]
fn notification_click_switches_server_and_channel_and_closes_settings() {
    let mut rt = with_notifiable_servers();
    click(&mut rt, SETTINGS_BUTTON_KEY);
    assert!(rt.state.settings().is_some());
    let mirante_id = named("Mirante");
    rt.update(|shell, tx| {
        shell.notification_clicked(
            ClickTarget {
                server: mirante_id.to_string(),
                user: Uuid::nil(),
                channel: Uuid::from_u128(3),
                message: Uuid::from_u128(99),
                is_dm: false,
            },
            tx,
        );
    });
    rt.frame(FRAME_S);
    assert!(rt.state.settings().is_none());
    assert_eq!(rt.state.navigation().selected().unwrap().id, mirante_id);
    assert_eq!(
        rt.state.navigation().selected_channel(),
        Some(uuid_id(3).as_str())
    );
}
