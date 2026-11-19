//! Device/backend eligibility uses committed policy; delivery visibility and ordinary All messages do not define badges.

use super::*;
use papo_api::wire::UpdateChannelUserSettingRequest;
use papo_sdk::{ChannelNotificationMode, NotificationSettings};

fn persisted(f: &Fixture, expected: &NotificationSettings) -> Result {
    let connection = rusqlite::Connection::open(f.profile.path().join("state.sqlite"))?;
    let json: Vec<u8> = connection.query_row(
        "SELECT settings FROM notification_state WHERE settings IS NOT NULL LIMIT 1",
        [],
        |row| row.get(0),
    )?;
    let actual: NotificationSettings = serde_json::from_slice(&json)?;
    assert!(
        actual == *expected,
        "device settings were not durably committed"
    );
    Ok(())
}

#[test]
fn committed_master_mentions_mute_and_channel_modes_recount_without_history_requests() -> Result {
    let f = Fixture::new(config())?;
    f.run(async {
        let session = account(&f, "badge-settings").await?;
        let mut changes = session.subscribe_badges();
        let direct = message(
            2,
            channel(),
            AUTHOR.parse()?,
            &format!("@mention(<@{}>)", session.account().user()),
        );
        load_and_anchor(
            &f,
            &session,
            channel(),
            vec![
                message(1, channel(), AUTHOR.parse()?, "anchor"),
                direct,
                message(3, channel(), AUTHOR.parse()?, "ordinary"),
                message(4, channel(), session.account().user(), "self @everyone"),
            ],
        )
        .await?;
        count(&mut changes, channel(), UnreadCount::Exact(1)).await?;
        let mut held = f
            .server()
            .hold_next_rest("GET", &format!("/channels/{CHANNEL}/messages"))
            .await?;
        let default = NotificationSettings::default();
        let mut disabled = Vec::new();
        let mut master = default.clone();
        master.enabled = false;
        disabled.push(master);
        let mut mentions = default.clone();
        mentions.mentions = false;
        disabled.push(mentions);
        let mut muted = default.clone();
        muted.muted.insert(channel());
        disabled.push(muted);
        let mut off = default.clone();
        off.channels.insert(channel(), ChannelNotificationMode::Off);
        disabled.push(off);
        for settings in disabled {
            session
                .notifications()
                .set_settings(settings.clone())
                .await?;
            persisted(&f, &settings)?;
            count(&mut changes, channel(), UnreadCount::Exact(0)).await?;
            session
                .notifications()
                .set_settings(default.clone())
                .await?;
            count(&mut changes, channel(), UnreadCount::Exact(1)).await?;
        }
        for mode in [
            ChannelNotificationMode::All,
            ChannelNotificationMode::Mentions,
        ] {
            let mut settings = default.clone();
            settings.channels.insert(channel(), mode);
            session
                .notifications()
                .set_settings(settings.clone())
                .await?;
            persisted(&f, &settings)?;
            // All changes delivery admission, not which messages are mention triggers.
            assert_eq!(
                changes.borrow().channels[&channel()].mention_count,
                UnreadCount::Exact(1)
            );
        }
        assert!(
            tokio::time::timeout(Duration::ZERO, held.wait_started())
                .await
                .is_err()
        );
        held.release();
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn decisive_disabled_policy_proves_zero_with_missing_parent_and_cold_read_evidence() -> Result {
    let f = Fixture::new(config())?;
    f.run(async {
        let session = account(&f, "badge-cold").await?;
        let mut changes = session.subscribe_badges();
        let mut reply = message(2, channel(), AUTHOR.parse()?, "@everyone");
        reply.reply_to = Some(message_id(99).to_string());
        f.server().seed_messages(CHANNEL, vec![reply]).await?;
        session
            .history(channel())
            .await?
            .page(HistoryQuery::default())
            .await?;
        count(&mut changes, channel(), UnreadCount::AtLeast(0)).await?;
        let mut settings = session.notifications().settings().await?;
        settings.enabled = false;
        session
            .notifications()
            .set_settings(settings.clone())
            .await?;
        persisted(&f, &settings)?;
        count(&mut changes, channel(), UnreadCount::Exact(0)).await?;
        settings.enabled = true;
        session.notifications().set_settings(settings).await?;
        count(&mut changes, channel(), UnreadCount::AtLeast(0)).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn dm_switch_controls_nonself_dm_but_self_messages_never_increment() -> Result {
    let f = Fixture::new(config())?;
    f.run(async {
        let first = account(&f, "badge-dm-first").await?;
        let second = account(&f, "badge-dm-second").await?;
        let conversation = first
            .operations()
            .open_direct_conversation(second.account().user())
            .await?
            .value;
        let dm: Uuid = conversation.id.parse()?;
        let mut changes = first.subscribe_badges();
        for value in [
            message(1, dm, second.account().user(), "anchor"),
            message(2, dm, second.account().user(), "direct"),
            message(3, dm, first.account().user(), "self"),
        ] {
            let id: Uuid = value.id.as_deref().ok_or("message id")?.parse()?;
            event(&first, "message", async {
                f.server()
                    .emit_user_event(
                        &first.account().user().to_string(),
                        papo_api::gateway::KnownEvent::Message(Box::new(
                            papo_api::gateway::MessageEvent {
                                id: value.id.ok_or("message id")?,
                                channel_id: dm.to_string(),
                                author_id: value.author_id.ok_or("message author")?,
                                content: value.content.ok_or("message content")?,
                                created_at: value.created_at.ok_or("message time")?,
                                ..Default::default()
                            },
                        )),
                    )
                    .await?;
                Ok(())
            })
            .await?;
            if id == message_id(1) {
                read_through(&first, dm, id).await?;
            }
        }
        let unread = count(&mut changes, dm, UnreadCount::Exact(1)).await?;
        assert!(unread.direct_message);
        let mut settings = first.notifications().settings().await?;
        settings.direct_messages = false;
        first.notifications().set_settings(settings.clone()).await?;
        count(&mut changes, dm, UnreadCount::Exact(0)).await?;
        settings.direct_messages = true;
        settings.mentions = false;
        first.notifications().set_settings(settings).await?;
        count(&mut changes, dm, UnreadCount::Exact(1)).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn backend_off_cannot_be_overridden_by_device_all() -> Result {
    let f = Fixture::new(config())?;
    f.run(async {
        let session = account(&f, "badge-backend").await?;
        let mut changes = session.subscribe_badges();
        load_and_anchor(
            &f,
            &session,
            channel(),
            vec![
                message(1, channel(), AUTHOR.parse()?, "anchor"),
                message(
                    2,
                    channel(),
                    AUTHOR.parse()?,
                    &format!("@mention(<@{}>)", session.account().user()),
                ),
            ],
        )
        .await?;
        count(&mut changes, channel(), UnreadCount::Exact(1)).await?;
        let mut settings = session.notifications().settings().await?;
        settings
            .channels
            .insert(channel(), ChannelNotificationMode::All);
        session.notifications().set_settings(settings).await?;
        session
            .operations()
            .update_channel_settings(
                channel(),
                UpdateChannelUserSettingRequest {
                    notification_settings: "off".into(),
                },
            )
            .await?;
        count(&mut changes, channel(), UnreadCount::Exact(0)).await?;
        session
            .operations()
            .update_channel_settings(
                channel(),
                UpdateChannelUserSettingRequest {
                    notification_settings: "only_mentions".into(),
                },
            )
            .await?;
        count(&mut changes, channel(), UnreadCount::Exact(1)).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
