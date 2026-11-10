//! Proves member events use one summary request, never list repaging or eager profile fetching.

use papo_api::{
    Uuid,
    gateway::{AvatarUpdateEvent, KnownEvent},
    wire::{RoleSummary, UserProfile},
};
use papo_test_server::Config;

use super::{
    fixture::{Fixture, Result},
    reconcile::{resources, signed_in},
};

fn member(index: u128) -> UserProfile {
    UserProfile {
        id: Some(Uuid::from_u128(0x2000_0000_0000_4000_8000_0000_0000_0000 + index).to_string()),
        username: Some(format!("member-{index}")),
        nickname: Some("original".into()),
        roles: Some(Vec::new()),
        ..UserProfile::default()
    }
}

fn member_updates() -> [(&'static str, UserProfile); 4] {
    let mut joined = member(300);
    joined.nickname = Some("joined".into());
    let mut role_added = joined.clone();
    role_added.nickname = Some("role-added".into());
    role_added.roles = Some(vec![RoleSummary {
        id: Some(Uuid::from_u128(700).to_string()),
        name: Some("updated-role".into()),
        ..RoleSummary::default()
    }]);
    let mut role_removed = role_added.clone();
    role_removed.nickname = Some("role-removed".into());
    role_removed.roles = Some(Vec::new());
    let mut avatar_changed = role_removed.clone();
    avatar_changed.nickname = Some("avatar-changed".into());
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
        let original_ids: Vec<_> = snapshots
            .borrow()
            .members
            .iter()
            .map(|user| user.id.clone())
            .collect();

        let mutations = member_updates();
        let target_id = Uuid::parse_str(mutations[0].1.id.as_deref().unwrap())?;
        for (kind, expected) in mutations {
            session.profiles(&[target_id, untouched_id]).await?;
            let before = fixture.server().member_request_counts().await;
            fixture.server().set_profile(expected.clone()).await?;
            let snapshot = resources(&mut snapshots, |snapshot| {
                snapshot
                    .members
                    .iter()
                    .any(|user| user.id == expected.id && user.nickname == expected.nickname)
                    && snapshot
                        .profiles
                        .iter()
                        .all(|profile| profile.id != expected.id)
            })
            .await?;
            assert_eq!(snapshot.members.len(), 207, "{kind} duplicated a member");
            assert_eq!(
                snapshot.members.last().unwrap().id,
                expected.id,
                "{kind} moved the joined user"
            );
            assert_eq!(
                snapshot
                    .members
                    .iter()
                    .take(original_ids.len())
                    .map(|user| user.id.clone())
                    .collect::<Vec<_>>(),
                original_ids,
                "{kind} changed unrelated membership"
            );
            let refreshed = snapshot
                .members
                .iter()
                .find(|user| user.id == expected.id)
                .unwrap();
            assert_eq!(
                refreshed
                    .roles
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|role| (&role.id, &role.name))
                    .collect::<Vec<_>>(),
                expected
                    .roles
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|role| (&role.id, &role.name))
                    .collect::<Vec<_>>(),
                "{kind} lost authoritative roles"
            );
            assert_eq!(
                snapshot.profiles.len(),
                1,
                "{kind} invalidated unrelated profiles"
            );
            assert_eq!(snapshot.profiles[0].id, untouched.id);
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
            snapshot.members.iter().all(|user| user.id != removed.id)
        })
        .await?;
        assert_eq!(snapshot.members.len(), 2);
        assert!(snapshot.members.iter().any(|user| user.id == retained.id));
        assert_eq!(snapshot.profiles.len(), 1);
        assert_eq!(snapshot.profiles[0].id, retained.id);
        let after = fixture.server().member_request_counts().await;
        assert_eq!(after.list_requests, before.list_requests);
        assert_eq!(after.summary_requests, before.summary_requests + 1);
        assert_eq!(after.profile_requests, before.profile_requests);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
