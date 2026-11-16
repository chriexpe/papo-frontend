//! Public badge counts over loopback history; rendering and delivery never decide eligibility.

use super::support::{Fixture, Result, channel};
use futures_util::StreamExt;
use papo_sdk::{Message, NotificationSettings, Session, UnreadCount, Uuid};
use papo_sdk_ui::{
    ChannelListService, ServerListService, Timeline, TimelineConfig, VisibilityContinuity,
};
use std::collections::BTreeSet;

const AUTHOR: &str = "20000000-0000-4000-8000-000000000001";

async fn until_mentions(session: &Session, target: Uuid, count: UnreadCount) -> Result {
    let mut reading = session.subscribe_reading();
    loop {
        if reading
            .borrow_and_update()
            .channels
            .get(&target)
            .is_some_and(|state| state.mention_count == count)
        {
            return Ok(());
        }
        reading.changed().await?;
    }
}

/// Seeds six messages: an own `@everyone`, a duplicated direct mention, a reply to the own
/// message, an owner `@everyone`, an unsupported role token and an ordinary row. Three count.
async fn seed_mixed_history(fixture: &Fixture, user: Uuid) -> Result<Vec<Uuid>> {
    let ids: Vec<_> = (1..=6).map(Uuid::from_u128).collect();
    let mention = papo_sdk::mentions::encode_user_mention(user);
    let content = [
        "@everyone".to_owned(),
        format!("`{mention}` {mention}"),
        String::new(),
        "@everyone".to_owned(),
        format!("@role(<@{}>)", Uuid::from_u128(8)),
        String::new(),
    ];
    let messages = ids
        .iter()
        .zip(content)
        .enumerate()
        .map(|(index, (id, content))| Message {
            id: Some(id.to_string()),
            channel_id: Some(channel().to_string()),
            author_id: Some(if index == 0 {
                user.to_string()
            } else {
                AUTHOR.into()
            }),
            reply_to: (index == 2).then(|| ids[0].to_string()),
            content: Some(content),
            created_at: Some(format!("2026-01-01T00:00:00.{index:06}Z")),
            ..Message::default()
        })
        .collect();
    fixture
        .server()
        .seed_messages(&channel().to_string(), messages)
        .await?;
    Ok(ids)
}

#[test]
fn public_badges_classify_history_recount_settings_and_clear_after_reading() -> Result {
    let fixture = Fixture::new(0, vec![])?;
    fixture.run(async {
        fixture
            .server()
            .set_server_info(serde_json::from_value(serde_json::json!({
                "id": "00000000-0000-4000-8000-000000000000", "name": "fixture",
                "icon_blob": null, "icon_format": "", "owner_id": AUTHOR, "public": true,
                "owner_username": "owner", "created_at": "2026-01-01T00:00:00Z",
                "role_count": 0, "member_count": 1, "channel_count": 0,
            }))?)
            .await;
        let session = fixture.signed_in("badge-policy").await?;
        let ids = seed_mixed_history(&fixture, session.account().user()).await?;
        session
            .notifications()
            .set_visible_channels(BTreeSet::from([channel()]))?;
        let timeline = Timeline::live(
            session.clone(),
            channel(),
            TimelineConfig {
                page_size: 50,
                ..TimelineConfig::default()
            },
        )
        .await?;
        until_mentions(&session, channel(), UnreadCount::Exact(3)).await?;
        let channels = ChannelListService::new(session.clone()).subscribe();
        assert_eq!(
            channels.snapshot[0].mention_count(),
            Some(UnreadCount::Exact(3))
        );
        let handle = fixture.client().handle();
        handle
            .set_active_account(session.account().server(), Some(session.account()))
            .await?;
        let mut badges = ServerListService::new(handle).subscribe_badges();
        assert_eq!(
            badges.snapshot.accounts[&session.account()]
                .unwrap()
                .mentions,
            UnreadCount::Exact(3)
        );
        assert_eq!(
            badges.snapshot.direct_messages.unwrap().mentions,
            UnreadCount::Exact(0)
        );

        let mut settings = NotificationSettings::default();
        settings.muted.insert(channel());
        session.notifications().set_settings(settings).await?;
        let muted = badges.stream.next().await.ok_or("badge stream closed")?;
        assert_eq!(
            muted.snapshot.accounts[&session.account()]
                .unwrap()
                .mentions,
            UnreadCount::Exact(0)
        );
        assert_eq!(
            muted.snapshot.servers[&session.account().server()]
                .unwrap()
                .mentions,
            UnreadCount::Exact(0)
        );
        session
            .notifications()
            .set_settings(NotificationSettings::default())
            .await?;
        let unmuted = badges.stream.next().await.ok_or("badge stream closed")?;
        assert_eq!(
            unmuted.snapshot.accounts[&session.account()]
                .unwrap()
                .mentions,
            UnreadCount::Exact(3)
        );
        let current = timeline.subscribe()?;
        timeline
            .report_visible(
                current.version,
                current.generation,
                &[ids[5]],
                VisibilityContinuity::Discontinuous,
            )
            .await?;
        until_mentions(&session, channel(), UnreadCount::Exact(0)).await?;
        let read = badges.stream.next().await.ok_or("badge stream closed")?;
        assert_eq!(
            read.snapshot.accounts[&session.account()].unwrap().mentions,
            UnreadCount::Exact(0)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn direct_message_eligibility_and_device_switch_stay_account_isolated() -> Result {
    let fixture = Fixture::new(0, vec![])?;
    fixture.run(async {
        let first = fixture.signed_in("badge-dm-first").await?;
        let second = fixture
            .signed_in_on(first.account().server(), "badge-dm-second")
            .await?;
        // The fake server stores history only for channel resources. Its authoritative
        // DM lists attach that existing history identity to both account-local peers.
        for (session, peer) in [(&first, &second), (&second, &first)] {
            fixture.server().set_direct_conversations(&session.account().user().to_string(), vec![
                serde_json::from_value(serde_json::json!({
                    "id": channel(), "user": { "id": peer.account().user(), "username": "peer" },
                    "created_at": "2026-01-01T00:00:00Z", "unread_count": 0,
                }))?
            ]).await?;
            let mut resources = session.subscribe_resources();
            loop {
                if resources
                    .borrow_and_update()
                    .dms
                    .iter()
                    .any(|dm| dm.id == channel().to_string())
                {
                    break;
                }
                resources.changed().await?;
            }
        }
        fixture
            .server()
            .seed_messages(
                &channel().to_string(),
                vec![Message {
                    id: Some(Uuid::from_u128(1).to_string()),
                    channel_id: Some(channel().to_string()),
                    author_id: Some(second.account().user().to_string()),
                    created_at: Some("2026-01-01T00:00:00.000001Z".into()),
                    content: Some(String::new()),
                    ..Message::default()
                }],
            )
            .await?;
        let first_timeline =
            Timeline::live(first.clone(), channel(), TimelineConfig::default()).await?;
        let second_timeline =
            Timeline::live(second.clone(), channel(), TimelineConfig::default()).await?;
        until_mentions(&first, channel(), UnreadCount::Exact(1)).await?;
        until_mentions(&second, channel(), UnreadCount::Exact(0)).await?;
        let mut badges = ServerListService::new(fixture.client().handle()).subscribe_badges();
        assert_eq!(
            badges.snapshot.direct_messages.unwrap().mentions,
            UnreadCount::Exact(1)
        );
        let settings = NotificationSettings {
            direct_messages: false,
            ..NotificationSettings::default()
        };
        first.notifications().set_settings(settings).await?;
        let muted = badges.stream.next().await.ok_or("badge stream closed")?;
        assert_eq!(
            muted.snapshot.direct_messages.unwrap().mentions,
            UnreadCount::Exact(0)
        );
        assert_eq!(
            second.reading_state(channel()).unwrap().mention_count,
            UnreadCount::Exact(0)
        );
        drop((first_timeline, second_timeline));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
