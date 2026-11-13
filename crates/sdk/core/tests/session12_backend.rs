//! Actual pinned Go notification/search/push contracts on disposable PostgreSQL; not a real FCM proof.
#![cfg(feature = "test-support")]
use papo_api::{
    MessageUpload, RestClient,
    wire::{CreateChannelRequest, LoginRequest, RegisterRequest},
};
use papo_sdk::{
    Client, ClientConfig, ConnectionState, DataRoot, PushDevice, PushRegistration, SearchQuery,
    Session, UnixMicros, Uuid, secret::test_store::ScriptedSecretStore,
};
use std::{sync::Arc, time::Duration};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
async fn online(session: &Session) -> Result {
    let mut connection = session.subscribe_connection();
    loop {
        if connection.borrow().generation == session.generation()
            && connection.borrow().state == ConnectionState::Online
        {
            return Ok(());
        }
        connection.changed().await?;
    }
}
#[test]
#[ignore = "requires isolated pinned backend and PostgreSQL; run scripts/papo-backend-proof.py"]
fn sdk_notifications_search_and_push_use_actual_pinned_handlers() -> Result {
    let address = std::env::var("PAPO_BACKEND_PROOF_ADDRESS")?;
    let password = std::env::var("PAPO_PROOF_PASSWORD")?;
    let directory = tempfile::tempdir()?;
    let mut client = Client::open(
        DataRoot::new(directory.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig {
            insecure_loopback: true,
            ..ClientConfig::default()
        },
    )?;
    client.block_on(async {
        tokio::time::timeout(Duration::from_secs(25), async {
            let (session, rest, channel, author) = prepare(&client, &address, &password).await?;
            prove_delivery_and_search(&client.handle(), &session, &rest, channel, author).await?;
            prove_push_and_reads(&session).await
        })
        .await?
    })??;
    let report = client.shutdown()?;
    assert!(report.state.finished && report.shared.finished);
    Ok(())
}

async fn prepare(
    client: &Client,
    address: &str,
    password: &str,
) -> Result<(Session, RestClient, Uuid, Uuid)> {
    let rest = RestClient::loopback(address)?;
    rest.login(&LoginRequest {
        username: "cursorproof".into(),
        password: password.to_owned(),
    })
    .await?;
    let author: Uuid = rest.whoami().await?.id.ok_or("author")?.parse()?;
    let channel: Uuid = rest
        .create_channel(&CreateChannelRequest {
            name: "notification-search-proof".into(),
            r#type: Some("text".into()),
            ..Default::default()
        })
        .await?
        .id
        .ok_or("channel")?
        .parse()?;
    let handle = client.handle();
    let server = handle.add_server(address, UnixMicros(0)).await?;
    let auth = handle.auth(server)?;
    auth.register(RegisterRequest {
        username: "notifyproof".into(),
        password: password.to_owned(),
    })
    .await?;
    let session = auth
        .login(
            LoginRequest {
                username: "notifyproof".into(),
                password: password.to_owned(),
            },
            UnixMicros(0),
        )
        .await?;
    online(&session).await?;
    handle
        .push_wake(
            session.account(),
            tokio::time::Instant::now() + Duration::from_secs(5),
        )
        .await?;
    Ok((session, rest, channel, author))
}
async fn prove_delivery_and_search(
    handle: &papo_sdk::ClientHandle,
    session: &Session,
    rest: &RestClient,
    channel: Uuid,
    author: Uuid,
) -> Result {
    let content = format!(
        "@mention(<@{}>) https://example.invalid",
        session.account().user()
    );
    let sent = rest
        .create_message(&MessageUpload {
            channel_id: &channel.to_string(),
            content: Some(&content),
            reply_to: None,
            embeds: None,
            attachments: &[],
        })
        .await?;
    let message: Uuid = sent.id.as_deref().ok_or("message")?.parse()?;
    let inbox = loop {
        let page = session.notifications().inbox(None).await?;
        if page.records.iter().any(|r| r.record.message_id == sent.id) {
            break page;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert!(inbox.records.iter().any(|r| r.actual_mention));
    let report = handle
        .push_wake(
            session.account(),
            tokio::time::Instant::now() + Duration::from_secs(5),
        )
        .await?;
    let mut deliveries = report.deliveries.len();
    if let Ok(delivery) = tokio::time::timeout(
        Duration::from_millis(250),
        session.notifications().next_delivery(),
    )
    .await
    {
        assert_eq!(delivery?.target.message, message);
        deliveries += 1;
    }
    assert_eq!(deliveries, 1);
    let query = SearchQuery {
        channel: Some(channel),
        author: Some(author),
        mention: Some(session.account().user()),
        has_link: true,
        contains_attachment: Some(false),
        ..Default::default()
    };
    let page = session.search(query)?.page(None).await?;
    assert_eq!(page.hits.len(), 1);
    assert_eq!(session.search_target(&page.hits[0])?.message, message);
    session.notifications().mark_read(&inbox.records).await?;
    Ok(())
}
async fn prove_push_and_reads(session: &Session) -> Result {
    let device = PushDevice {
        token: "synthetic-sdk-proof-device".into(),
        platform: "android".into(),
        provider: "fcm".into(),
        device_name: None,
    };
    assert_eq!(
        session
            .notifications()
            .register_push(device.clone())
            .await?,
        PushRegistration::Registered
    );
    assert_eq!(
        session
            .notifications()
            .unregister_push_device(device)
            .await?,
        PushRegistration::Removed
    );
    assert!(
        session
            .notifications()
            .inbox(None)
            .await?
            .records
            .iter()
            .all(|row| row.record.read == Some(true))
    );
    Ok(())
}
