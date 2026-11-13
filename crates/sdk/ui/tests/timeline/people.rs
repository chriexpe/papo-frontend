//! Public people/activity/capability lifecycles reuse the existing loopback fixture without native UI.
use super::support::{Fixture, Result, channel};
use futures_util::StreamExt;
use papo_sdk::{ConnectionState, UserProfile, Uuid};
use papo_sdk_ui::{ActivityService, CapabilityService, DirectMessageService, PeopleService};

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
                nickname: Some("first".into()),
                ..UserProfile::default()
            })
            .await?;
        let session = fixture.signed_in("people-views").await?;
        let service = PeopleService::new(session.clone());
        let mut members = service.members();
        let mut rows = members.snapshot.clone();
        let own = rows
            .iter()
            .find(|item| item.id == session.account().user().to_string())
            .unwrap()
            .clone();
        service.load_profiles(&[user]).await?;
        let before = fixture.server().member_request_counts().await;
        fixture
            .server()
            .set_profile(UserProfile {
                id: Some(user.to_string()),
                username: Some("view-member".into()),
                avatar_format: Some("webp".into()),
                nickname: Some("second".into()),
                ..UserProfile::default()
            })
            .await?;
        loop {
            let update = members.stream.next().await.ok_or("member stream closed")?;
            for diff in update.diffs {
                diff.apply(&mut rows);
            }
            if rows.iter().any(|item| {
                item.id == user.to_string() && item.nickname.as_deref() == Some("second")
            }) {
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
                .all(|item| item.member.id != user.to_string())
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
        let service = ActivityService::new(session.clone(), channel());
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
                assert_eq!(view.typing.len(), 0);
                assert_eq!(view.presence.len(), 0);
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
        assert!(service.report_typing().is_err());
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
        assert!(papo_sdk_ui::capabilities(&image, Uuid::from_u128(7), dm_id).is_err());
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
