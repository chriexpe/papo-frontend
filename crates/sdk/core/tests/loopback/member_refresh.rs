//! Proves member events use one summary request, never list repaging or eager profile fetching.

use papo_api::{
    Uuid,
    gateway::{AvatarUpdateEvent, KnownEvent},
    wire::{RoleSummary, UserProfile},
};
use papo_test_server::Config;
use std::sync::Arc;

use super::{
    fixture::{Fixture, Result},
    reconcile::{resources, signed_in},
};

fn member(index: u128) -> UserProfile {
    UserProfile {
        id: Some(Uuid::from_u128(0x2000_0000_0000_4000_8000_0000_0000_0000 + index).to_string()),
        username: Some(format!("member-{index}")),
        nickname: "original".into(),
        roles: Some(Vec::new()),
        ..UserProfile::default()
    }
}

fn member_updates() -> [(&'static str, UserProfile); 4] {
    let mut joined = member(300);
    joined.nickname = "joined".into();
    let mut role_added = joined.clone();
    role_added.nickname = "role-added".into();
    role_added.roles = Some(vec![RoleSummary {
        id: Some(Uuid::from_u128(700).to_string()),
        name: Some("updated-role".into()),
        ..RoleSummary::default()
    }]);
    let mut role_removed = role_added.clone();
    role_removed.nickname = "role-removed".into();
    role_removed.roles = Some(Vec::new());
    let mut avatar_changed = role_removed.clone();
    avatar_changed.nickname = "avatar-changed".into();
    avatar_changed.avatar_format = Some("webp".into());
    [
        ("user_join", joined),
        ("role_add", role_added),
        ("role_remove", role_removed),
        ("avatar_update", avatar_changed),
    ]
}

#[test]
fn each_member_event_fetches_one_summary_without_repaging_a_large_roster() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        for index in 0..205 {
            fixture.server().set_profile(member(index)).await?;
        }
        let untouched = member(0);
        let untouched_id = Uuid::parse_str(untouched.id.as_deref().unwrap())?;
        let session = signed_in(&fixture, "member-cost").await?;
        assert_eq!(session.subscribe_resources().borrow().members.len(), 206);
        assert_eq!(
            fixture.server().member_request_counts().await.list_requests,
            3
        );
        let mut snapshots = session.subscribe_resources();
        let original_ids: Vec<_> = snapshots.borrow().members.iter().collect();

        let mutations = member_updates();
        let target_id = Uuid::parse_str(mutations[0].1.id.as_deref().unwrap())?;
        for (kind, expected) in mutations {
            session.profiles(&[target_id, untouched_id]).await?;
            let before = fixture.server().member_request_counts().await;
            fixture.server().set_profile(expected.clone()).await?;
            let snapshot =
                resources(&mut snapshots, |snapshot| {
                    snapshot.users.get(target_id).is_some_and(|user| {
                        user.nickname.as_deref() == expected.nickname.as_deref()
                    }) && snapshot.profiles.get(target_id).is_none()
                })
                .await?;
            assert_eq!(snapshot.members.len(), 207, "{kind} duplicated a member");
            assert_eq!(
                snapshot.members.iter().last().unwrap(),
                target_id,
                "{kind} moved the joined user"
            );
            assert_eq!(
                snapshot
                    .members
                    .iter()
                    .take(original_ids.len())
                    .collect::<Vec<_>>(),
                original_ids,
                "{kind} changed unrelated membership"
            );
            let refreshed = snapshot.users.get(target_id).ok_or("target summary")?;
            let expected_roles: Vec<Uuid> = expected
                .roles
                .as_ref()
                .unwrap()
                .iter()
                .map(|role| role.id.as_deref().unwrap().parse())
                .collect::<std::result::Result<_, _>>()?;
            assert_eq!(
                refreshed.roles.as_ref(),
                Some(&expected_roles),
                "{kind} lost observed assignments"
            );
            assert_eq!(
                snapshot.profiles.len(),
                1,
                "{kind} invalidated unrelated profiles"
            );
            assert_eq!(snapshot.profiles.ids().next(), Some(untouched_id));
            let after = fixture.server().member_request_counts().await;
            assert_eq!(
                after.list_requests, before.list_requests,
                "{kind} repaged members"
            );
            assert_eq!(
                after.summary_requests,
                before.summary_requests + 1,
                "{kind} did not fetch exactly one summary batch"
            );
            assert_eq!(
                after.profile_requests, before.profile_requests,
                "{kind} fetched a profile eagerly"
            );
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn absent_summary_removes_only_that_member_and_invalidates_their_profile() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let removed = member(1);
        let retained = member(2);
        let removed_id = Uuid::parse_str(removed.id.as_deref().unwrap())?;
        let retained_id = Uuid::parse_str(retained.id.as_deref().unwrap())?;
        fixture.server().set_profile(removed.clone()).await?;
        fixture.server().set_profile(retained.clone()).await?;
        let session = signed_in(&fixture, "absent-summary").await?;
        session.profiles(&[removed_id, retained_id]).await?;
        let mut snapshots = session.subscribe_resources();
        let before = fixture.server().member_request_counts().await;
        assert!(
            fixture
                .server()
                .remove_member(removed.id.as_deref().unwrap())
                .await
        );
        fixture
            .server()
            .emit_event(KnownEvent::AvatarUpdate(Box::new(AvatarUpdateEvent {
                user_id: removed_id.to_string(),
            })))
            .await?;
        let snapshot = resources(&mut snapshots, |snapshot| {
            !snapshot.members.contains(removed_id)
        })
        .await?;
        assert_eq!(snapshot.members.len(), 2);
        assert!(snapshot.members.contains(retained_id));
        assert_eq!(snapshot.profiles.len(), 1);
        assert_eq!(snapshot.profiles.ids().next(), Some(retained_id));
        let after = fixture.server().member_request_counts().await;
        assert_eq!(after.list_requests, before.list_requests);
        assert_eq!(after.summary_requests, before.summary_requests + 1);
        assert_eq!(after.profile_requests, before.profile_requests);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn gateway_and_operation_refresh_join_the_same_user_in_each_relationship() -> Result {
    use papo_api::gateway::UserJoinEvent;
    use papo_api::wire::{DirectConversation, UserSummary};
    use papo_sdk::operations::MetadataScope;
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let peer = member(21);
        let id: Uuid = peer.id.as_deref().ok_or("peer ID")?.parse()?;
        fixture.server().set_profile(peer).await?;
        let session = signed_in(&fixture, "src-equality").await?;
        let owner = session.account().user().to_string();
        let user = UserSummary {
            id: Some(id.to_string()),
            ..UserSummary::default()
        };
        fixture
            .server()
            .set_blocks(&owner, vec![user.clone()])
            .await;
        let dm_id = Uuid::from_u128(55);
        fixture
            .server()
            .set_direct_conversations(
                &owner,
                vec![DirectConversation {
                    id: dm_id.to_string(),
                    user,
                    ..Default::default()
                }],
            )
            .await?;
        let mut watch = session.subscribe_resources();
        resources(&mut watch, |root| {
            root.dms.get(dm_id).is_some() && root.blocks.contains(id)
        })
        .await?;
        let mut first = None;
        let role = Uuid::from_u128(79);
        for gateway in [true, false] {
            if !gateway {
                session.reconnect()?;
                super::reconcile::online(&session).await?;
            }
            session.profiles(&[id]).await?;
            let batch = serde_json::json!([{"id":id,"nickname":"updated","roles":[{"id":role}]}]);
            fixture
                .server()
                .queue_operation_response("POST", "/users/user_summary_batch", 200, batch)
                .await?;
            if gateway {
                fixture
                    .server()
                    .emit_event(KnownEvent::UserJoin(Box::new(UserJoinEvent {
                        user_id: id.to_string(),
                    })))
                    .await?;
            } else {
                let result = session
                    .operations()
                    .refresh_metadata(MetadataScope::Member(id))
                    .await?;
                assert_eq!(result.value, papo_sdk::RefreshStatus::Updated);
            }
            let root = resources(&mut watch, |root| {
                root.users
                    .get(id)
                    .is_some_and(|user| user.nickname.as_deref() == Some("updated"))
            })
            .await?;
            let dm = root.dms.get(dm_id).ok_or("DM")?;
            let summary = root.users.get_arc(dm.peer_id).ok_or("DM summary")?;
            assert!(root.members.contains(id) && root.blocks.contains(id));
            assert!(Arc::ptr_eq(&summary, &root.users.get_arc(id).unwrap()));
            assert_eq!(summary.roles.as_deref(), Some([role].as_slice()));
            assert!(root.profiles.get(id).is_none());
            let observed = (summary, root.members.iter().collect::<Vec<_>>());
            if let Some(previous) = first.take() {
                assert_eq!(previous, observed);
            } else {
                first = Some(observed);
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
