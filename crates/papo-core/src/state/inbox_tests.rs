use super::*;
use serde_json::json;
fn notice(id: &str) -> models::Notification {
    serde_json::from_value(json!({"id":id,"message_id":"m","channel_id":"c","created_at":"2026-10-23T12:00:00.123456789Z"})).unwrap()
}
#[test]
fn parity_inbox_rejects_stale_account_pages_and_deleted_message_replay() {
    let mut store = Store {
        me: "me".into(),
        ..Default::default()
    };
    let old = store.inbox.begin(false).unwrap();
    let epoch = store.cache_restore_epoch();
    store.apply(Update::Session(None));
    store.me = "me".into();
    let current = store.inbox.begin(false).unwrap();
    store.apply(Update::NotificationInbox {
        owner: "me".into(),
        epoch,
        flight: old,
        result: Ok(models::NotificationList {
            notifications: vec![notice("stale")],
            has_more: false,
        }),
    });
    assert!(store.inbox.items().is_empty());
    store.apply(Update::Event(Box::new(Event::MessageDeleted {
        id: "m".into(),
        channel_id: "c".into(),
    })));
    store.apply(Update::NotificationInbox {
        owner: "me".into(),
        epoch: store.cache_restore_epoch(),
        flight: current,
        result: Ok(models::NotificationList {
            notifications: vec![notice("deleted")],
            has_more: true,
        }),
    });
    assert!(store.inbox.items().is_empty());
    assert_eq!(store.inbox.cursor.as_ref().unwrap().id, "deleted");
}
#[test]
fn parity_late_settings_ack_cannot_replace_another_account() {
    let mut store = Store {
        me: "replacement".into(),
        ..Default::default()
    };
    store.apply(Update::UserSettings(Box::new(models::UserSettings {
        user_id: "old".into(),
        version: 1,
        config: models::UserConfig::default(),
        updated_at: None,
    })));
    assert!(store.user_settings.is_none());
}
