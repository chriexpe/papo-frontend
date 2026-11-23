//! Public people/activity/capability lifecycles reuse the existing loopback fixture without native UI.
use super::support::{Fixture, Result, channel};
use futures_util::StreamExt;
use papo_sdk::{ConnectionState, UserProfile, Uuid};
use papo_sdk_ui::{
    ActivityService, CapabilityService, DirectMessageService, PeopleService, PresenceMark, person,
    requested_profile,
};

#[test]
fn member_updates_keep_unrelated_rows_and_invalidate_requested_profiles_without_roster_fetches()
-> Result {
    let fixture = Fixture::new(0, vec![])?;
    fixture.run(async {
        let user = Uuid::from_u128(0x2000_0000_0000_4000_8000_0000_0000_0042);
        fixture
            .server()
            .set_profile(UserProfile {
                id: Some(user.to_string()),
                username: Some("view-member".into()),
                nickname: "first".into(),
                description: Some("about".into()),
                created_at: Some("2025-03-03T12:00:00Z".into()),
                ..UserProfile::default()
            })
            .await?;
        let session = fixture.signed_in("people-views").await?;
        let service = PeopleService::new(session.clone());
        let mut members = service.members();
        let mut rows = members.snapshot.clone();
        let own = rows
            .iter()
            .find(|item| item.id == session.account().user())
            .unwrap()
            .clone();
        let image = session.subscribe_resources().borrow().clone();
        assert!(
            requested_profile(&image, user).is_none(),
            "not requested yet"
        );
        service.load_profiles(&[user]).await?;
        let image = session.subscribe_resources().borrow().clone();
        let profile = requested_profile(&image, user).ok_or("requested profile missing")?;
        assert_eq!(profile.description.as_deref(), Some("about"));
        assert_eq!(
            profile.member_since.as_deref(),
            Some("2025-03-03T12:00:00Z")
        );
        assert_eq!(person(&image, user), profile.member);
        let before = fixture.server().member_request_counts().await;
        fixture
            .server()
            .set_profile(UserProfile {
                id: Some(user.to_string()),
                username: Some("view-member".into()),
                avatar_format: Some("webp".into()),
                nickname: "second".into(),
                ..UserProfile::default()
            })
            .await?;
        loop {
            let update = members.stream.next().await.ok_or("member stream closed")?;
            for diff in update.diffs {
                diff.apply(&mut rows);
            }
            if rows
                .iter()
                .any(|item| item.id == user && item.nickname.as_deref() == Some("second"))
            {
                break;
            }
        }
        assert!(std::sync::Arc::ptr_eq(
            &own,
            rows.iter().find(|item| item.id == own.id).unwrap()
        ));
        let after = fixture.server().member_request_counts().await;
        assert_eq!(after.list_requests, before.list_requests);
        assert_eq!(after.profile_requests, before.profile_requests);
        assert_eq!(after.summary_requests, before.summary_requests + 1);
        assert!(
            service
                .profiles()
                .snapshot
                .iter()
                .all(|item| item.member.id != user)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn connection_loss_clears_activity_and_capabilities_are_unavailable_until_reconciled() -> Result {
    let fixture = Fixture::new(0, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("activity-view").await?;
        let service = ActivityService::new(session.clone());
        let mut activity = service.subscribe();
        let capabilities = CapabilityService::new(session.clone());
        let mut caps = capabilities.subscribe(channel());
        assert!(caps.snapshot.channel.is_ok());
        fixture.server().set_gateway_accepting(false).await;
        fixture.server().disconnect_gateways();
        loop {
            let view = activity
                .stream
                .next()
                .await
                .ok_or("activity stream closed")?;
            if session.subscribe_connection().borrow().state != ConnectionState::Online {
                assert!(!view.live);
                assert_eq!(view.typing.len(), 0);
                assert_eq!(view.presence.len(), 0);
                assert_eq!(view.presence_mark(session.account().user()), None);
                break;
            }
        }
        while caps
            .stream
            .next()
            .await
            .ok_or("capability stream closed")?
            .channel
            .is_ok()
        {}
        assert!(capabilities.channel(channel()).is_err());
        assert!(service.report_typing(channel()).is_err());
        fixture.server().set_gateway_accepting(true).await;
        super::support::online(&session).await?;
        assert!(capabilities.channel(channel()).is_ok());
        session.auth().logout().await?;
        assert!(activity.stream.next().await.is_none());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn unified_dm_list_removes_revoked_account_rows() -> Result {
    let fixture = Fixture::new(0, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("dm-views").await?;
        let dm = papo_sdk::DirectConversation {
            id: Uuid::from_u128(9).to_string(),
            user: papo_sdk::UserSummary {
                id: Some(Uuid::from_u128(8).to_string()),
                ..Default::default()
            },
            created_at: "2026-01-01T00:00:00Z".into(),
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            unread_count: 0,
        };
        let mut source = session.subscribe_resources();
        fixture
            .server()
            .set_direct_conversations(&session.account().user().to_string(), vec![dm])
            .await?;
        while source.borrow_and_update().dms.is_empty() {
            source.changed().await?;
        }
        let mut list = DirectMessageService::new(vec![session.clone()]).subscribe();
        let mut rows = list.snapshot.clone();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id.account, session.account());
        let dm_id = Uuid::from_u128(9);
        let caps = CapabilityService::new(session.clone()).channel(dm_id)?;
        assert!(caps.value.read && caps.value.send && caps.value.attach);
        let image = session.subscribe_resources().borrow().clone();
        // The plain function reads a DM from the same evidence as the service.
        assert_eq!(
            papo_sdk_ui::capabilities(&image, session.account().user(), dm_id)?,
            caps.value
        );
        session.auth().logout().await?;
        while !rows.is_empty() {
            for diff in list
                .stream
                .next()
                .await
                .ok_or("DM source ended before removal")?
                .diffs
            {
                diff.apply(&mut rows);
            }
        }
        assert!(list.stream.next().await.is_none());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn a_journaled_dm_update_cannot_restore_a_conversation_removed_by_newer_rest_data() -> Result {
    let fixture = Fixture::new(0, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("stale-dm").await?;
        let dm = papo_sdk::DirectConversation {
            id: Uuid::from_u128(19).to_string(),
            user: papo_sdk::UserSummary {
                id: Some(Uuid::from_u128(18).to_string()),
                ..Default::default()
            },
            created_at: "2026-01-01T00:00:00Z".into(),
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            unread_count: 0,
        };
        let account = session.account().user().to_string();
        fixture
            .server()
            .set_direct_conversations(&account, vec![dm.clone()])
            .await?;
        let mut resources = session.subscribe_resources();
        while resources.borrow_and_update().dms.is_empty() {
            resources.changed().await?;
        }
        let channel = Uuid::from_u128(19);
        let capabilities = CapabilityService::new(session.clone());
        assert!(capabilities.channel(channel)?.value.send);
        // Models a reverse block or hide: the own block list remains empty, but /dms excludes the row.
        fixture
            .server()
            .set_direct_conversations(&account, Vec::new())
            .await?;
        let mut hold = fixture
            .server()
            .hold_next_rest("GET", "/users/blocks")
            .await?;
        session.reconnect()?;
        hold.wait_started().await?;
        let mut events = session.subscribe_events();
        fixture
            .server()
            .emit_event(serde_json::from_value(
                serde_json::json!({"type": "dm_update", "dm": dm}),
            )?)
            .await?;
        hold.release();
        loop {
            if events.next().await?.value.kind() == "dm_update" {
                break;
            }
        }
        super::support::online(&session).await?;
        let image = session.subscribe_resources().borrow().clone();
        assert_eq!(image.blocks.len(), 0);
        assert_eq!(image.dms.len(), 0);
        assert!(capabilities.channel(channel).is_err());
        assert_eq!(
            DirectMessageService::new(vec![session])
                .subscribe()
                .snapshot
                .len(),
            0
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn account_activity_marks_presence_and_lists_others_typing_per_channel() -> Result {
    let fixture = Fixture::new(0, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("activity-marks").await?;
        let mut activity = ActivityService::new(session.clone()).subscribe();
        let lia = Uuid::from_u128(0x2000_0000_0000_4000_8000_0000_0000_0051);
        let yuki = Uuid::from_u128(0x2000_0000_0000_4000_8000_0000_0000_0052);
        let gone = Uuid::from_u128(0x2000_0000_0000_4000_8000_0000_0000_0053);
        let elsewhere = Uuid::from_u128(0x2000_0000_0000_4000_8000_0000_0000_0099);
        let events = [
            serde_json::json!({"type": "presence_update", "user_id": lia.to_string(),
                "status": "away", "status_message": "de férias"}),
            serde_json::json!({"type": "presence_update", "user_id": yuki.to_string(),
                "status": "a-future-status"}),
            serde_json::json!({"type": "typing", "channel_id": channel().to_string(),
                "user_id": yuki.to_string(), "is_typing": true}),
            serde_json::json!({"type": "typing", "channel_id": channel().to_string(),
                "user_id": lia.to_string(), "is_typing": true}),
            serde_json::json!({"type": "typing", "channel_id": elsewhere.to_string(),
                "user_id": lia.to_string(), "is_typing": true}),
            serde_json::json!({"type": "typing", "channel_id": channel().to_string(),
                "user_id": session.account().user().to_string(), "is_typing": true}),
        ];
        for event in events {
            fixture
                .server()
                .emit_event(serde_json::from_value(event)?)
                .await?;
        }
        let mut view = activity.snapshot.clone();
        while view.typing_in(elsewhere).is_empty() || view.typing_in(channel()).len() < 2 {
            view = activity
                .stream
                .next()
                .await
                .ok_or("activity stream closed")?;
        }
        assert!(view.live);
        assert_eq!(view.presence_mark(lia), Some(PresenceMark::Away));
        assert_eq!(view.status_phrase(lia), Some("de férias"));
        assert_eq!(
            view.presence_mark(yuki),
            Some(PresenceMark::Available),
            "an unknown status still means connected"
        );
        assert_eq!(view.presence_mark(gone), Some(PresenceMark::Offline));
        let here: Vec<Uuid> = view
            .typing_in(channel())
            .iter()
            .map(|item| item.user)
            .collect();
        assert_eq!(here, [lia, yuki], "others only, in a stable order");
        assert_eq!(view.typing_in(elsewhere).len(), 1);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn direct_access_answers_from_the_lists_and_asks_only_for_a_conversation_they_left_out() -> Result {
    use papo_sdk::operations::social::DirectAccess;
    let fixture = Fixture::new(0, vec![])?;
    fixture.run(async {
        let peer = Uuid::from_u128(28);
        fixture
            .server()
            .set_profile(UserProfile {
                id: Some(peer.to_string()),
                username: Some("direct-peer".into()),
                ..UserProfile::default()
            })
            .await?;
        let session = fixture.signed_in("direct-access").await?;
        let conversation = Uuid::from_u128(29);
        let dm = papo_sdk::DirectConversation {
            id: conversation.to_string(),
            user: papo_sdk::UserSummary {
                id: Some(peer.to_string()),
                ..Default::default()
            },
            created_at: "2026-01-01T00:00:00Z".into(),
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            unread_count: 0,
        };
        let account = session.account().user().to_string();
        fixture
            .server()
            .set_direct_conversations(&account, vec![dm])
            .await?;
        let mut resources = session.subscribe_resources();
        while resources.borrow_and_update().dms.is_empty() {
            resources.changed().await?;
        }
        let operations = session.operations();
        assert_eq!(
            operations.direct_access(conversation, peer).await?,
            DirectAccess::Open
        );
        assert_eq!(
            operations
                .direct_access(Uuid::from_u128(30), Uuid::from_u128(31))
                .await?,
            DirectAccess::Gone,
            "a conversation the account is not in"
        );
        operations.block_user(peer).await?;
        while !resources.borrow_and_update().blocks.contains(peer) {
            resources.changed().await?;
        }
        assert_eq!(
            operations.direct_access(conversation, peer).await?,
            DirectAccess::YouBlocked
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn account_activity_publishes_only_a_view_that_changed() -> Result {
    let fixture = Fixture::new(0, vec![])?;
    fixture.run(async {
        let session = fixture.signed_in("activity-quiet").await?;
        let mut activity = ActivityService::new(session.clone()).subscribe();
        let lia = Uuid::from_u128(0x2000_0000_0000_4000_8000_0000_0000_0061);
        // The account's own typing changes the SDK's memory, not the view.
        for event in [
            serde_json::json!({"type": "typing", "channel_id": channel().to_string(),
                "user_id": session.account().user().to_string(), "is_typing": true}),
            serde_json::json!({"type": "presence_update", "user_id": lia.to_string(),
                "status": "busy"}),
        ] {
            fixture
                .server()
                .emit_event(serde_json::from_value(event)?)
                .await?;
        }
        let view = activity
            .stream
            .next()
            .await
            .ok_or("activity stream closed")?;
        assert_eq!(
            view.presence_mark(lia),
            Some(PresenceMark::Busy),
            "the first view published is the one with news"
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
