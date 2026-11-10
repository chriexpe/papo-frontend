//! Proves public on-demand profile batching and event invalidation; protocol validation stays in API tests.

use papo_api::{
    Uuid,
    gateway::{AvatarUpdateEvent, KnownEvent},
    wire::UserProfile,
};
use papo_sdk::ClientError;
use papo_test_server::Config;

use super::{
    fixture::{Fixture, Result},
    reconcile::{online, resources, signed_in},
};

fn profile(index: u128) -> UserProfile {
    UserProfile {
        id: Some(Uuid::from_u128(index + 0x2000_0000_0000_4000_8000_0000_0000_0000).to_string()),
        username: Some(format!("profile-{index}")),
        nickname: Some("before-invalidation".into()),
        ..UserProfile::default()
    }
}

#[test]
fn profiles_are_not_eagerly_loaded_and_requests_larger_than_fifty_are_batched() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let mut ids = Vec::new();
        for index in 0..51 {
            let profile = profile(index);
            ids.push(Uuid::parse_str(profile.id.as_deref().unwrap())?);
            fixture.server().set_profile(profile).await?;
        }
        let session = signed_in(&fixture, "profile-batches").await?;
        let mut snapshots = session.subscribe_resources();
        assert!(snapshots.borrow().profiles.is_empty());
        assert!(snapshots.borrow().members.len() >= 51);
        let response = session.profiles(&ids).await?;
        assert_eq!(response.generation, session.generation());
        assert_eq!(response.value.len(), 51);
        for id in &ids {
            assert!(
                response
                    .value
                    .iter()
                    .any(|profile| profile.id.as_deref() == Some(id.to_string().as_str()))
            );
        }
        let snapshot = resources(&mut snapshots, |snapshot| snapshot.profiles.len() == 51).await?;
        assert_eq!(snapshot.generation, response.generation);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn avatar_event_invalidates_only_the_requested_user_and_next_request_reads_fresh_profile() -> Result
{
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let first = profile(1);
        let second = profile(2);
        fixture.server().set_profile(first.clone()).await?;
        fixture.server().set_profile(second.clone()).await?;
        let session = signed_in(&fixture, "profile-inval").await?;
        let first_id = Uuid::parse_str(first.id.as_deref().unwrap())?;
        let second_id = Uuid::parse_str(second.id.as_deref().unwrap())?;
        session.profiles(&[first_id, second_id]).await?;
        let mut snapshots = session.subscribe_resources();
        let mut changed = first.clone();
        changed.nickname = Some("after-invalidation".into());
        fixture.server().set_profile(changed).await?;
        fixture
            .server()
            .emit_event(KnownEvent::AvatarUpdate(Box::new(AvatarUpdateEvent {
                user_id: first_id.to_string(),
            })))
            .await?;
        let snapshot = resources(&mut snapshots, |snapshot| {
            snapshot
                .profiles
                .iter()
                .all(|profile| profile.id != first.id)
        })
        .await?;
        assert_eq!(snapshot.profiles.len(), 1);
        assert_eq!(snapshot.profiles[0].id, second.id);
        let fresh = session.profiles(&[first_id]).await?;
        assert_eq!(
            fresh.value[0].nickname.as_deref(),
            Some("after-invalidation")
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn profile_batch_captured_before_reconnect_cannot_publish_in_the_new_generation() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let before = profile(9);
        let id = Uuid::parse_str(before.id.as_deref().unwrap())?;
        fixture.server().set_profile(before.clone()).await?;
        let session = signed_in(&fixture, "stale-profile").await?;
        let mut hold = fixture.server().hold_next_rest("POST", "/users/profile_batch").await?;
        let requested = [id];
        let request = session.profiles(&requested);
        tokio::pin!(request);
        tokio::select! {
            started = hold.wait_started() => started?,
            result = &mut request => { result?; return Err("profile request bypassed hold".into()); }
        }
        let mut changed = before;
        changed.nickname = Some("new-profile-generation".into());
        fixture.server().set_profile(changed).await?;
        session.reconnect()?;
        assert!(matches!(request.await, Err(ClientError::Superseded)));
        online(&session).await?;
        hold.release();
        assert!(session.subscribe_resources().borrow().profiles.is_empty());
        let fresh = session.profiles(&[id]).await?;
        assert_eq!(fresh.value[0].nickname.as_deref(), Some("new-profile-generation"));
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
