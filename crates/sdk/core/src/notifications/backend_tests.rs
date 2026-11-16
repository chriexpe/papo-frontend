//! The backend policy cache wakes a badge recount exactly when an eligibility switch changes.

use super::backend::remember;
use papo_api::wire::{Notifications, UserConfig, WhoamiResponseSettings};

fn settings(version: i64, notifications: Notifications) -> WhoamiResponseSettings {
    WhoamiResponseSettings {
        version: Some(version),
        config: Some(UserConfig {
            notifications: Some(notifications),
            ..Default::default()
        }),
    }
}

#[test]
fn only_eligibility_switches_signal_a_recount_and_older_versions_are_ignored() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = super::tests::fixture(directory.path());
    let inner = &session.inner;
    let generation = session.generation();
    let mut signal = inner.notifications.policy_changes.subscribe();
    let cached_mentions = || {
        let cached = inner.notifications.backend.lock().unwrap();
        cached.as_ref().unwrap().config.mentions
    };

    remember(
        inner,
        generation,
        Some(settings(1, Notifications::default())),
    )
    .unwrap();
    assert!(!signal.has_changed().unwrap(), "an identical config");

    let muted_mentions = Notifications {
        mentions: Some(false),
        ..Default::default()
    };
    remember(inner, generation, Some(settings(2, muted_mentions.clone()))).unwrap();
    assert!(signal.has_changed().unwrap(), "mentions decide eligibility");
    signal.borrow_and_update();

    // Sound and preview shape delivery only, so a badge count does not depend on them.
    let quieter = Notifications {
        sound: Some(false),
        message_preview: Some(false),
        ..muted_mentions
    };
    remember(inner, generation, Some(settings(3, quieter))).unwrap();
    assert!(!signal.has_changed().unwrap(), "presentation fields only");

    let reopened = Notifications {
        mentions: Some(true),
        ..Default::default()
    };
    remember(inner, generation, Some(settings(1, reopened))).unwrap();
    assert!(!signal.has_changed().unwrap(), "an older version is stale");
    assert_eq!(cached_mentions(), Some(false));

    let off = Notifications {
        enabled: Some(false),
        mentions: Some(false),
        ..Default::default()
    };
    remember(inner, generation, Some(settings(4, off))).unwrap();
    assert!(
        signal.has_changed().unwrap(),
        "the master switch decides eligibility"
    );
    client.shutdown().unwrap();
}
