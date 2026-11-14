//! Operation acknowledgments, partial failures and lifecycle races over loopback; Go proofs specify authorization.
#![cfg(feature = "test-support")]
mod fixture;
use fixture::{Fixture, Result, login, register};
use papo_sdk::{
    ClientError, OperationError, RefreshFault, RefreshStatus, Session, UnixMicros, Uuid,
    operations::channels::CreateChannelRequest,
};
use serde_json::json;

#[path = "operation_cases/lifecycle.rs"]
mod lifecycle;
#[path = "operation_cases/personal.rs"]
mod personal;
#[path = "operation_cases/roles.rs"]
mod roles;

async fn setup(f: &Fixture) -> Result<(Session, Session, Uuid)> {
    let client = f.client().handle();
    let server = client
        .add_server(&f.server().base_url(), UnixMicros(0))
        .await?;
    let auth = client.auth(server)?;
    register(&auth, "operation-a").await?;
    let first = login(&auth, "operation-a").await?;
    fixture::online(&first).await?;
    let auth = client.auth(server)?;
    register(&auth, "operation-b").await?;
    let second = login(&auth, "operation-b").await?;
    fixture::online(&second).await?;
    let created = first
        .operations()
        .create_channel(CreateChannelRequest {
            name: "fixture".into(),
            r#type: Some("text".into()),
            ..Default::default()
        })
        .await?;
    let channel = created.value.id.ok_or("missing channel")?.parse()?;
    Ok((first, second, channel))
}

#[test]
fn failed_refresh_keeps_acknowledgment_and_history() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, _, channel) = setup(&f).await?;
        let before = session.subscribe_resources().borrow().channels.clone();
        f.server()
            .queue_operation_response("GET", "/channels", 503, json!({}))
            .await?;
        let receipt = session
            .operations()
            .create_channel(CreateChannelRequest {
                name: "accepted".into(),
                r#type: Some("text".into()),
                ..Default::default()
            })
            .await?;
        assert!(receipt.value.id.is_some());
        assert_eq!(
            receipt.refresh,
            RefreshStatus::Warning(RefreshFault::Request)
        );
        assert_eq!(
            session.subscribe_resources().borrow().channels.len(),
            before.len()
        );
        assert!(
            session
                .subscribe_resources()
                .borrow()
                .channels
                .iter()
                .any(|c| c.id.as_deref().and_then(|id| id.parse().ok()) == Some(channel))
        );
        receipt.validate(&session)?;
        assert_eq!(
            session
                .operations()
                .refresh_metadata(papo_sdk::operations::MetadataScope::Channels)
                .await?
                .value,
            RefreshStatus::Updated
        );
        let requests = f.server().take_operation_requests().await;
        assert_eq!(
            requests
                .iter()
                .filter(|(method, path, _, _)| method == "POST" && path == "/channels")
                .count(),
            2
        );
        assert!(
            session
                .subscribe_resources()
                .borrow()
                .channels
                .iter()
                .any(|channel| channel.id == receipt.value.id)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn missing_required_list_is_refresh_warning_without_revocation() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, _, channel) = setup(&f).await?;
        f.server()
            .queue_operation_response("GET", "/channels", 200, json!({}))
            .await?;
        let receipt = session
            .operations()
            .create_channel(CreateChannelRequest {
                name: "accepted".into(),
                ..Default::default()
            })
            .await?;
        assert_eq!(
            receipt.refresh,
            RefreshStatus::Warning(RefreshFault::Request)
        );
        assert!(
            session
                .subscribe_resources()
                .borrow()
                .channels
                .iter()
                .any(|c| c.id.as_deref().and_then(|id| id.parse().ok()) == Some(channel))
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn batch_keeps_success_denial_unknown_and_does_not_retry() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, _, _) = setup(&f).await?;
        let ids = [Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3)];
        for (id, status) in ids.into_iter().zip([204, 403, 503]) {
            f.server()
                .queue_operation_response("DELETE", &format!("/emojis/{id}"), status, json!({}))
                .await?;
        }
        let results = session
            .operations()
            .delete_emojis_batch(ids.to_vec())
            .await?;
        assert!(results[0].is_ok());
        assert!(matches!(results[1], Err(OperationError::Session(_))));
        assert!(matches!(results[2], Err(OperationError::Unknown(_))));
        let requests = f.server().take_operation_requests().await;
        assert_eq!(
            requests
                .iter()
                .filter(|(method, path, _, _)| method == "DELETE" && path.starts_with("/emojis/"))
                .count(),
            3
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn reconnect_fences_dispatched_mutation_and_remaining_batch() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, _, _) = setup(&f).await?;
        let ids = [Uuid::from_u128(10), Uuid::from_u128(11)];
        let path = format!("/emojis/{}", ids[0]);
        f.server()
            .queue_operation_response("DELETE", &path, 204, json!({}))
            .await?;
        let mut held = f.server().hold_next_rest("DELETE", &path).await?;
        let operations = session.operations();
        let task = tokio::spawn(async move { operations.delete_emojis_batch(ids.to_vec()).await });
        held.wait_started().await?;
        session.reconnect()?;
        let results = task.await??;
        assert!(matches!(results[0], Err(OperationError::Unknown(_))));
        assert!(matches!(
            results[1],
            Err(OperationError::Session(ClientError::Superseded))
        ));
        held.release();
        fixture::online(&session).await?;
        let requests = f.server().take_operation_requests().await;
        assert_eq!(
            requests
                .iter()
                .filter(|(method, path, _, _)| method == "DELETE" && path.starts_with("/emojis/"))
                .count(),
            1
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn concurrent_mutations_refresh_in_acknowledged_order() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, _, _) = setup(&f).await?;
        let mut held = f.server().hold_next_rest("POST", "/channels").await?;
        let first = session.operations();
        let one = tokio::spawn(async move {
            first
                .create_channel(CreateChannelRequest {
                    name: "first".into(),
                    ..Default::default()
                })
                .await
        });
        held.wait_started().await?;
        let second = session.operations();
        let two = tokio::spawn(async move {
            second
                .create_channel(CreateChannelRequest {
                    name: "second".into(),
                    ..Default::default()
                })
                .await
        });
        held.release();
        let first = one.await??;
        let second = two.await??;
        assert_eq!(first.refresh, RefreshStatus::Updated);
        assert_eq!(second.refresh, RefreshStatus::Updated);
        let image = session.subscribe_resources().borrow().clone();
        for id in [first.value.id, second.value.id] {
            assert!(image.channels.iter().any(|c| c.id == id));
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn concurrent_gateway_publication_preserves_newer_image() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, _, _) = setup(&f).await?;
        let mut held = f.server().hold_next_rest("GET", "/channels").await?;
        let operations = session.operations();
        let task = tokio::spawn(async move {
            operations
                .create_channel(CreateChannelRequest {
                    name: "accepted".into(),
                    ..Default::default()
                })
                .await
        });
        held.wait_started().await?;
        let created = f.server().create_channel("newer-live").await?;
        let mut resources = session.subscribe_resources();
        loop {
            if resources
                .borrow()
                .channels
                .iter()
                .any(|c| c.id == created.id)
            {
                break;
            }
            resources.changed().await?;
        }
        held.release();
        let receipt = task.await??;
        assert_eq!(
            receipt.refresh,
            RefreshStatus::Warning(RefreshFault::ConcurrentChange)
        );
        assert!(
            resources
                .borrow()
                .channels
                .iter()
                .any(|c| c.id == created.id)
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn completed_operations_need_no_replay_after_client_restart() -> Result {
    let mut f = Fixture::new(papo_test_server::Config::default())?;
    let account = f.run(async {
        let (session, _, _) = setup(&f).await?;
        Ok::<_, Box<dyn std::error::Error>>(session.account())
    })??;
    assert!(f.store.calls(papo_sdk::secret::test_store::Operation::Read) > 0);
    f.reopen()?;
    f.run(async {
        let session = f.client().handle().session(account)?;
        fixture::online(&session).await?;
        let requests = f.server().take_operation_requests().await;
        assert_eq!(
            requests
                .iter()
                .filter(|(method, path, _, _)| method == "POST" && path == "/channels")
                .count(),
            1
        );
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}
