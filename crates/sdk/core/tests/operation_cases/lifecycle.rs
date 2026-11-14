//! Accepted mutation refresh cancellation and account replacement never publish stale state.

use super::{Fixture, Result, setup};
use papo_sdk::{
    OperationError, RefreshFault, RefreshStatus, operations::channels::CreateChannelRequest,
};
use serde_json::json;

#[test]
fn reconnect_during_refresh_keeps_known_server_success() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, _, _) = setup(&f).await?;
        let generation = session.generation();
        let mut held = f.server().hold_next_rest("GET", "/channels").await?;
        let operations = session.operations();
        let task = tokio::spawn(async move {
            operations
                .create_channel(CreateChannelRequest {
                    name: "accepted-before-reconnect".into(),
                    ..Default::default()
                })
                .await
        });
        held.wait_started().await?;
        session.reconnect()?;
        let receipt = task.await??;
        assert_eq!(receipt.generation, generation);
        assert!(receipt.value.id.is_some());
        assert_eq!(
            receipt.refresh,
            RefreshStatus::Warning(RefreshFault::Superseded)
        );
        assert!(receipt.validate(&session).is_err());
        held.release();
        crate::fixture::online(&session).await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn logout_during_dispatch_revokes_old_handle_and_never_replays_mutation() -> Result {
    let f = Fixture::new(papo_test_server::Config::default())?;
    f.run(async {
        let (session, other, _) = setup(&f).await?;
        let mut held = f.server().hold_next_rest("POST", "/webhooks").await?;
        f.server().queue_operation_response("POST", "/webhooks", 201, json!({"id":"00000000-0000-4000-8000-000000000001", "token":"synthetic-ephemeral-credential"})).await?;
        let operations = session.operations();
        let task = tokio::spawn(async move {operations.create_webhook(papo_api::wire::CreateWebhookRequest {channel_id:papo_sdk::Uuid::from_u128(1).to_string(), ..Default::default()}).await});
        held.wait_started().await?;
        session.auth().logout().await?;
        assert!(session.is_closed());
        assert!(matches!(task.await?, Err(OperationError::Unknown(_))));
        held.release();
        let requests = f.server().take_operation_requests().await;
        assert_eq!(requests.iter().filter(|(method,path,_,_)|method == "POST" && path == "/webhooks").count(),1);
        assert!(other.operations().roles().await.is_ok());
        assert!(session.operations().roles().await.is_err());
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    f.finish()
}

#[test]
fn client_shutdown_during_refresh_preserves_the_acknowledged_response() -> Result {
    let mut f = Fixture::new(papo_test_server::Config::default())?;
    let (session, mut held) = f.run(async {
        let (session, _, _) = setup(&f).await?;
        let held = f.server().hold_next_rest("GET", "/channels").await?;
        Ok::<_, Box<dyn std::error::Error>>((session, held))
    })??;
    let operations = session.operations();
    // A host executor calls the SDK; SDK work still uses its one owned runtime.
    let caller = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let request = std::thread::spawn(move || {
        caller.block_on(operations.create_channel(CreateChannelRequest {
            name: "confirmed-before-shutdown".into(),
            ..Default::default()
        }))
    });
    f.run(held.wait_started())??;
    f.close_client()?;
    let receipt = request.join().map_err(|_| "host caller failed")??;
    assert!(receipt.value.id.is_some());
    assert_eq!(
        receipt.refresh,
        RefreshStatus::Warning(RefreshFault::Closed)
    );
    assert!(receipt.validate(&session).is_err());
    held.release();
    f.finish()
}
