//! Exercises public resource reconciliation over held loopback responses; journal internals stay in core unit tests.

use std::sync::Arc;

use papo_sdk::{ConnectionState, Generation, ResourceSnapshot, Session, UnixMicros};
use papo_test_server::Config;
use tokio::sync::watch;

use super::fixture::{Fixture, Result, login, register, wait_for};

pub(super) async fn online(session: &Session) -> Result<Generation> {
    let mut status = session.subscribe_connection();
    Ok(wait_for(&mut status, |status| {
        status.state == ConnectionState::Online && status.generation == session.generation()
    })
    .await?
    .generation)
}

pub(super) async fn signed_in(fixture: &Fixture, username: &str) -> Result<Session> {
    let handle = fixture.client().handle();
    let server = handle
        .add_server(&fixture.server().base_url(), UnixMicros(0))
        .await?;
    let auth = handle.auth(server)?;
    register(&auth, username).await?;
    let session = login(&auth, username).await?;
    online(&session).await?;
    Ok(session)
}

pub(super) async fn resources(
    receiver: &mut watch::Receiver<Arc<ResourceSnapshot>>,
    predicate: impl Fn(&ResourceSnapshot) -> bool,
) -> Result<Arc<ResourceSnapshot>> {
    loop {
        let snapshot = receiver.borrow_and_update().clone();
        if predicate(&snapshot) {
            return Ok(snapshot);
        }
        receiver.changed().await?;
    }
}

#[test]
fn reconcile_replays_channel_mutations_after_a_captured_channels_response() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let renamed = fixture.server().create_channel("before-rename").await?;
        let deleted = fixture.server().create_channel("before-delete").await?;
        let session = signed_in(&fixture, "journal-user").await?;
        let old_generation = session.generation();
        let mut snapshots = session.subscribe_resources();
        let mut hold = fixture.server().hold_next_rest("GET", "/channels").await?;
        session.reconnect()?;
        hold.wait_started().await?;
        let generation = session.generation();
        assert!(generation.get() > old_generation.get());
        assert_ne!(
            session.subscribe_connection().borrow().state,
            ConnectionState::Online
        );
        let created = fixture.server().create_channel("during-reconcile").await?;
        assert!(
            fixture
                .server()
                .rename_channel(renamed.id.as_deref().unwrap(), "after-rename")
                .await
        );
        assert!(
            fixture
                .server()
                .delete_channel(deleted.id.as_deref().unwrap())
                .await
        );
        hold.release();
        let snapshot = resources(&mut snapshots, |snapshot| {
            snapshot.generation == generation
                && snapshot
                    .channels
                    .iter()
                    .any(|channel| channel.id == created.id)
                && snapshot.channels.iter().any(|channel| {
                    channel.id == renamed.id && channel.name.as_deref() == Some("after-rename")
                })
                && snapshot
                    .channels
                    .iter()
                    .all(|channel| channel.id != deleted.id)
        })
        .await?;
        assert_eq!(online(&session).await?, generation);
        assert!(snapshot.version > 0);
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn held_old_generation_response_cannot_replace_new_generation_channels() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let channel = fixture.server().create_channel("old-generation").await?;
        let session = signed_in(&fixture, "stale-resource").await?;
        let mut hold = fixture.server().hold_next_rest("GET", "/channels").await?;
        session.reconnect()?;
        hold.wait_started().await?;
        let superseded = session.generation();
        assert!(
            fixture
                .server()
                .rename_channel(channel.id.as_deref().unwrap(), "new-generation")
                .await
        );
        session.reconnect()?;
        let generation = online(&session).await?;
        assert!(generation.get() > superseded.get());
        hold.release();
        let mut snapshots = session.subscribe_resources();
        let marker = fixture.server().create_channel("after-release").await?;
        let snapshot = resources(&mut snapshots, |snapshot| {
            snapshot
                .channels
                .iter()
                .any(|channel| channel.id == marker.id)
        })
        .await?;
        assert_eq!(snapshot.generation, generation);
        assert!(
            snapshot
                .channels
                .iter()
                .any(|current| current.id == channel.id
                    && current.name.as_deref() == Some("new-generation"))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn disconnected_channel_changes_replace_the_same_resource_subscription() -> Result {
    let fixture = Fixture::new(Config::default())?;
    fixture.run(async {
        let renamed = fixture.server().create_channel("connected").await?;
        let deleted = fixture.server().create_channel("deleted-offline").await?;
        let session = signed_in(&fixture, "offline-user").await?;
        let first = session.generation();
        let mut snapshots = session.subscribe_resources();
        let mut states = session.subscribe_connection();
        fixture.server().set_gateway_accepting(false).await;
        fixture.server().disconnect_gateways();
        wait_for(&mut states, |status| {
            matches!(status.state, ConnectionState::Backoff { .. })
        })
        .await?;
        let created = fixture.server().create_channel("created-offline").await?;
        assert!(
            fixture
                .server()
                .rename_channel(renamed.id.as_deref().unwrap(), "renamed-offline")
                .await
        );
        assert!(
            fixture
                .server()
                .delete_channel(deleted.id.as_deref().unwrap())
                .await
        );
        fixture.server().set_gateway_accepting(true).await;
        let next = online(&session).await?;
        assert!(next.get() > first.get());
        let snapshot = resources(&mut snapshots, |snapshot| {
            snapshot.generation == next
                && snapshot
                    .channels
                    .iter()
                    .any(|channel| channel.id == created.id)
        })
        .await?;
        assert!(
            snapshot
                .channels
                .iter()
                .all(|channel| channel.id != deleted.id)
        );
        assert!(
            snapshot
                .channels
                .iter()
                .any(|channel| channel.id == renamed.id
                    && channel.name.as_deref() == Some("renamed-offline"))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
