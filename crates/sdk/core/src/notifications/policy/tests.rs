//! Pure trigger-evidence and committed channel-policy regressions; storage signals are tested separately.

use super::{ChannelPolicy, Classification, NotificationSettings, Provenance, classify};
use crate::{ChannelNotificationMode, Message, Uuid};
use papo_api::wire::Notifications;

const ACCOUNT: Uuid = Uuid::from_u128(1);
const CHANNEL: Uuid = Uuid::from_u128(2);
const OTHER: Uuid = Uuid::from_u128(3);

fn message() -> Message {
    Message {
        author_id: Some(OTHER.to_string()),
        content: Some(String::new()),
        ..Default::default()
    }
}

fn policy(settings: &NotificationSettings, backend: &Notifications, dm: bool) -> ChannelPolicy {
    ChannelPolicy::new(settings, backend, CHANNEL, dm, None)
}

#[test]
fn direct_mention_beyond_preview_is_independent_of_missing_parent_and_everyone_authority() {
    let mut item = message();
    item.reply_to = Some(OTHER.to_string());
    item.content = Some(format!(
        "{} `@MENTION(<@{ACCOUNT}>)` @everyone",
        " ".repeat(513)
    ));
    assert_eq!(
        classify(&item, ACCOUNT, false, None),
        Classification::KnownTrigger(Provenance::DirectMention)
    );
}

#[test]
fn reply_requires_valid_held_parent_authorship_not_its_identifier_alone() {
    let mut item = message();
    item.reply_to = Some(OTHER.to_string());
    assert_eq!(
        classify(&item, ACCOUNT, false, None),
        Classification::Unknown
    );
    assert_eq!(
        classify(&item, ACCOUNT, false, Some(ACCOUNT)),
        Classification::KnownTrigger(Provenance::ReplyToAccount)
    );
    assert_eq!(
        classify(&item, ACCOUNT, false, Some(OTHER)),
        Classification::KnownNonTrigger
    );
    item.reply_to = Some(String::new());
    assert_eq!(
        classify(&item, ACCOUNT, false, Some(ACCOUNT)),
        Classification::Unknown
    );
}

#[test]
fn everyone_is_historically_unknown_but_null_author_and_webhook_cannot_broadcast() {
    let mut item = message();
    item.content = Some("`@EVERYONE`".into());
    assert_eq!(
        classify(&item, ACCOUNT, false, None),
        Classification::Unknown
    );
    item.author_id = None;
    item.webhook_id = Some(OTHER.to_string());
    assert_eq!(
        classify(&item, ACCOUNT, false, None),
        Classification::KnownNonTrigger
    );
    item.webhook_id = None;
    assert_eq!(
        classify(&item, ACCOUNT, false, None),
        Classification::KnownNonTrigger
    );
    item.reply_to = Some(OTHER.to_string());
    assert_eq!(
        classify(&item, ACCOUNT, false, None),
        Classification::Unknown
    );
}

#[test]
fn user_author_with_webhook_flag_does_not_disprove_everyone_but_direct_stays_positive() {
    let mut item = message();
    item.webhook_id = Some(OTHER.to_string());
    item.content = Some("@everyone".into());
    assert_eq!(
        classify(&item, ACCOUNT, false, None),
        Classification::Unknown
    );
    item.content = Some(format!(
        "@everyone {}",
        crate::mentions::encode_user_mention(ACCOUNT)
    ));
    assert_eq!(
        classify(&item, ACCOUNT, false, None),
        Classification::KnownTrigger(Provenance::DirectMention)
    );
}

#[test]
fn webhook_direct_mentions_remain_positive() {
    let mut item = message();
    item.author_id = None;
    item.webhook_id = Some(OTHER.to_string());
    item.content = Some(crate::mentions::encode_user_mention(ACCOUNT));
    assert_eq!(
        classify(&item, ACCOUNT, false, None),
        Classification::KnownTrigger(Provenance::DirectMention)
    );
}

#[test]
fn non_self_dm_is_positive_and_own_message_is_negative_even_with_every_trigger() {
    let mut item = message();
    assert_eq!(
        classify(&item, ACCOUNT, true, None),
        Classification::KnownTrigger(Provenance::DirectMessage)
    );
    item.author_id = Some(ACCOUNT.to_string());
    item.reply_to = Some(OTHER.to_string());
    item.content = Some(format!(
        "@everyone {}",
        crate::mentions::encode_user_mention(ACCOUNT)
    ));
    assert_eq!(
        classify(&item, ACCOUNT, true, Some(ACCOUNT)),
        Classification::KnownNonTrigger
    );
}

#[test]
fn role_tokens_are_not_triggers_and_missing_content_is_not_a_negative() {
    let mut item = message();
    item.content = Some(format!("<@&{ACCOUNT}> @role({ACCOUNT}) @everyone_extra"));
    assert_eq!(
        classify(&item, ACCOUNT, false, None),
        Classification::KnownNonTrigger
    );
    item.content = None;
    assert_eq!(
        classify(&item, ACCOUNT, false, None),
        Classification::Unknown
    );
}

#[test]
fn all_mode_delivers_ordinary_messages_without_counting_them_as_badge_triggers() {
    let settings = NotificationSettings::default();
    let backend = Notifications::default();
    let all = ChannelPolicy::new(&settings, &backend, CHANNEL, false, Some("all"));
    assert!(all.delivery_eligible(false));
    assert_eq!(
        all.badge_eligible(Classification::KnownNonTrigger),
        Some(false)
    );
    assert_eq!(all.badge_eligible(Classification::Unknown), None);
    assert_eq!(
        all.badge_eligible(Classification::KnownTrigger(Provenance::DirectMention)),
        Some(true)
    );
}

#[test]
fn disabled_master_mute_and_either_off_mode_decisively_filter_unknowns() {
    let backend = Notifications::default();
    let mut settings = NotificationSettings {
        enabled: false,
        ..Default::default()
    };
    let master_off = policy(&settings, &backend, false);
    settings.enabled = true;
    settings.muted.insert(CHANNEL);
    let muted = policy(&settings, &backend, false);
    settings.muted.clear();
    settings
        .channels
        .insert(CHANNEL, ChannelNotificationMode::Off);
    let device_off = policy(&settings, &backend, false);
    settings
        .channels
        .insert(CHANNEL, ChannelNotificationMode::All);
    let backend_off = ChannelPolicy::new(&settings, &backend, CHANNEL, false, Some("off"));
    let account_off = policy(
        &settings,
        &Notifications {
            enabled: Some(false),
            ..Default::default()
        },
        false,
    );
    for disabled in [master_off, muted, device_off, backend_off, account_off] {
        assert!(disabled.badges_disabled());
        assert_eq!(
            disabled.badge_eligible(Classification::Unknown),
            Some(false)
        );
        assert!(!disabled.delivery_eligible(true));
    }
}

#[test]
fn mentions_switch_filters_badges_even_when_all_mode_can_still_deliver() {
    let mut settings = NotificationSettings::default();
    settings
        .channels
        .insert(CHANNEL, ChannelNotificationMode::All);
    settings.mentions = false;
    let device_mentions_off = policy(&settings, &Notifications::default(), false);
    settings.mentions = true;
    let backend_mentions_off = policy(
        &settings,
        &Notifications {
            mentions: Some(false),
            ..Default::default()
        },
        false,
    );
    for policy in [device_mentions_off, backend_mentions_off] {
        assert!(policy.badges_disabled());
        assert_eq!(policy.badge_eligible(Classification::Unknown), Some(false));
        assert!(policy.delivery_eligible(false));
    }
}

#[test]
fn dm_switch_is_independent_of_mentions_and_dm_delivery_does_not_require_inbox_trigger() {
    let mut settings = NotificationSettings {
        mentions: false,
        ..Default::default()
    };
    let backend = Notifications {
        mentions: Some(false),
        ..Default::default()
    };
    let enabled = policy(&settings, &backend, true);
    assert!(enabled.delivery_eligible(false));
    assert_eq!(
        enabled.badge_eligible(Classification::KnownTrigger(Provenance::DirectMessage)),
        Some(true)
    );
    settings.direct_messages = false;
    let disabled = policy(&settings, &backend, true);
    assert!(disabled.badges_disabled());
    assert!(!disabled.delivery_eligible(true));
}

#[test]
fn authoritative_trigger_is_not_downgraded_by_unknown_historical_classification() {
    let mut item = message();
    item.content = Some("@everyone".into());
    assert_eq!(
        classify(&item, ACCOUNT, false, None),
        Classification::Unknown
    );
    let policy = policy(
        &NotificationSettings::default(),
        &Notifications::default(),
        false,
    );
    assert_eq!(policy.badge_eligible(Classification::Unknown), None);
    assert!(policy.delivery_eligible(true));
    assert!(!policy.delivery_eligible(false));
}
