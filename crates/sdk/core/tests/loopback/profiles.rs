//! Proves public on-demand profile batching and event invalidation; protocol validation stays in API tests.

use papo_api::{
    Uuid,
    gateway::{AvatarUpdateEvent, KnownEvent},
    wire::UserProfile,
};
use papo_sdk::{Backoff, ClientError, ConnectionState, HistoryError};
use papo_test_server::Config;

use super::{
    fixture::{Fixture, Result, wait_for},
    reconcile::{online, resources, signed_in},
};

fn profile(index: u128) -> UserProfile {
    UserProfile {
        id: Some(Uuid::from_u128(index + 0x2000_0000_0000_4000_8000_0000_0000_0000).to_string()),
        username: Some(format!("profile-{index}")),
        nickname: "before-invalidation".into(),
        ..UserProfile::default()
    }
}

#[test]
fn first_profile_request_after_restart_preserves_other_profiles_without_a_history_grant() -> Result
{
    let backoff = std::time::Duration::from_secs(3600);
    let mut fixture = Fixture::with_backoff(
        Config::default(),
        Backoff {
            initial: backoff,
            maximum: backoff,
        },
    )?;
    let mut first = profile(71);
    let second = profile(72);
    let first_id = Uuid::parse_str(first.id.as_deref().unwrap())?;
    let second_id = Uuid::parse_str(second.id.as_deref().unwrap())?;
    let (account, channel) = fixture.run(async {
        let channel = fixture.server().create_channel("cold-profiles").await?;
        let channel = Uuid::parse_str(channel.id.as_deref().unwrap())?;
        fixture.server().set_profile(first.clone()).await?;
        fixture.server().set_profile(second).await?;
        let session = signed_in(&fixture, "cold-profile").await?;
        session.profiles(&[first_id, second_id]).await?;
        session.history(channel).await?;
        fixture.server().set_gateway_accepting(false).await;
        Ok::<_, Box<dyn std::error::Error>>((session.account(), channel))
    })??;

    fixture.reopen()?;
    let mut complete = fixture.run(async {
        let session = fixture.client().handle().session(account)?;
        wait_for(&mut session.subscribe_connection(), |status| {
            matches!(status.state, ConnectionState::Backoff { .. })
        })
        .await?;
        assert_eq!(session.subscribe_resources().borrow().version, 0);
        first.description = Some("fresh detail".into());
        fixture.server().set_profile(first).await?;
        session.profiles(&[first_id]).await?;
        let display = session.subscribe_resources().borrow().clone();
        assert!(display.profiles.get(second_id).is_some());
        assert!(display.channels.get(channel).is_some());
        assert!(!display.channel_census_complete);
        assert!(matches!(
            session.history(channel).await,
            Err(HistoryError::Unverified)
        ));
        fixture.server().set_gateway_accepting(true).await;
        Ok::<_, Box<dyn std::error::Error>>(
            fixture.server().hold_next_rest("GET", "/channels").await?,
        )
    })??;

    // Hold the first complete fetch so this image can only come from the switched revision.
    fixture.reopen()?;
    fixture.run(async {
        complete.wait_started().await?;
        let session = fixture.client().handle().session(account)?;
        let display = session.subscribe_resources().borrow().clone();
        assert_eq!(
            display
                .profiles
                .get(first_id)
                .unwrap()
                .description
                .as_deref(),
            Some("fresh detail")
        );
        assert!(display.profiles.get(second_id).is_some());
        assert!(!display.channel_census_complete);
        assert!(matches!(
            session.history(channel).await,
            Err(HistoryError::Unverified)
        ));
        complete.release();
        online(&session).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
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
        changed.nickname = "after-invalidation".into();
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
                .all(|profile| profile.id != first_id)
        })
        .await?;
        assert_eq!(snapshot.profiles.len(), 1);
        assert_eq!(snapshot.profiles.ids().next(), Some(second_id));
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
        changed.nickname = "new-profile-generation".into();
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

#[test]
fn profile_admission_is_bounded_and_duplicate_ids_are_requested_only_once() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let profile = profile(42);
        let id = Uuid::parse_str(profile.id.as_deref().unwrap())?;
        fixture.server().set_profile(profile).await?;
        let session = signed_in(&fixture, "bounded-profiles").await?;
        let before = fixture.server().member_request_counts().await;
        assert!(matches!(
            session.profiles(&vec![id; 1001]).await,
            Err(ClientError::ProfileLimit)
        ));
        let after = fixture.server().member_request_counts().await;
        assert_eq!(before.profile_requests, after.profile_requests);
        let response = session.profiles(&[id, id, id]).await?;
        assert_eq!(response.value.len(), 1);
        assert_eq!(
            fixture
                .server()
                .member_request_counts()
                .await
                .profile_requests,
            before.profile_requests + 1
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn avatar_bytes_are_returned_to_the_caller_but_never_retained_in_shared_profiles() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let mut user = profile(99);
        let id = Uuid::parse_str(user.id.as_deref().unwrap())?;
        user.avatar_blob = Some("A".repeat(2 * 1024 * 1024));
        user.avatar_format = Some("webp".into());
        fixture.server().set_profile(user).await?;
        let session = signed_in(&fixture, "profile-blobs").await?;
        let response = session.profiles(&[id]).await?;
        assert_eq!(
            response.value[0].avatar_blob.as_ref().map(String::len),
            Some(2 * 1024 * 1024)
        );
        let image = session.subscribe_resources().borrow().clone();
        let retained = image.profiles.get(id).ok_or("profile details")?;
        assert_eq!(retained.avatar_format.as_deref(), Some("webp"));
        assert_eq!(
            image
                .users
                .get(id)
                .and_then(|user| user.nickname.as_deref()),
            Some("before-invalidation")
        );
        let response = session.profiles(&[id]).await?;
        let next = session.subscribe_resources().borrow().clone();
        assert!(std::sync::Arc::ptr_eq(
            &image.profiles.get_arc(id).unwrap(),
            &next.profiles.get_arc(id).unwrap()
        ));
        assert_eq!(
            response.value[0].avatar_blob.as_ref().map(String::len),
            Some(2 * 1024 * 1024)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
