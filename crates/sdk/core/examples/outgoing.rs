//! Runnable headless outgoing proof; native composers and platform credential stores belong to clients.

use papo_sdk::{
    Client, ClientConfig, ConnectionState, DataRoot, DraftBody, LoginRequest, MessageTarget,
    OperationKind, OperationState, OutgoingAttachment, OutgoingMessage, RegisterRequest,
    UnixMicros, Uuid, secret::test_store::ScriptedSecretStore,
};
use papo_test_server::{Config, TestServer};
use std::{error::Error, sync::Arc, time::Duration};

fn main() -> Result<(), Box<dyn Error>> {
    let fixture_runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()?;
    let fixture = if std::env::var_os("PAPO_ADDRESS").is_none() {
        Some(fixture_runtime.block_on(TestServer::start(Config::default()))?)
    } else {
        None
    };
    let address = fixture.as_ref().map_or_else(
        || std::env::var("PAPO_ADDRESS"),
        |server| Ok(server.base_url()),
    )?;
    let username = if fixture.is_some() {
        "outgoing-proof".into()
    } else {
        std::env::var("PAPO_USERNAME")?
    };
    let password = if fixture.is_some() {
        (0..20_u8)
            .map(|n| char::from(b'A' + n))
            .chain(['!'])
            .collect()
    } else {
        std::env::var("PAPO_PASSWORD")?
    };
    let channel_name = if fixture.is_some() {
        "outgoing-proof".into()
    } else {
        std::env::var("PAPO_CHANNEL_NAME")?
    };
    if let Some(server) = &fixture {
        fixture_runtime.block_on(server.create_channel(&channel_name))?;
    }
    let root = tempfile::tempdir()?;
    let mut client = Client::open(
        DataRoot::new(root.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig {
            insecure_loopback: std::env::var_os("PAPO_LOOPBACK_HTTP").is_some()
                || fixture.is_some(),
            ..Default::default()
        },
    )?;
    let result = client.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(30),
            prove(
                &client,
                &address,
                username,
                password,
                &channel_name,
                fixture.is_some(),
            ),
        )
        .await?
    })?;
    let report = client.shutdown()?;
    assert!(report.state.finished && report.shared.finished && report.reads_drained);
    if let Some(server) = fixture {
        fixture_runtime.block_on(server.shutdown())?;
    }
    result
}

async fn prove(
    client: &Client,
    address: &str,
    username: String,
    password: String,
    channel_name: &str,
    register: bool,
) -> Result<(), Box<dyn Error>> {
    let handle = client.handle();
    let server = handle.add_server(address, UnixMicros(0)).await?;
    let auth = handle.auth(server)?;
    if register {
        auth.register(RegisterRequest {
            username: username.clone(),
            password: password.clone(),
        })
        .await?;
    }
    let session = auth
        .login(LoginRequest { username, password }, UnixMicros(0))
        .await?;
    let mut connection = session.subscribe_connection();
    loop {
        if connection.borrow_and_update().state == ConnectionState::Online {
            break;
        }
        connection.changed().await?;
    }
    let channel = session
        .subscribe_resources()
        .borrow()
        .channels
        .iter()
        .find(|c| c.name.as_deref() == Some(channel_name))
        .and_then(|c| c.id.as_ref())
        .ok_or("proof channel unavailable")?
        .parse()?;
    exercise(session, channel).await
}

async fn exercise(session: papo_sdk::Session, channel: Uuid) -> Result<(), Box<dyn Error>> {
    let queue = session.outgoing();
    let revision = queue
        .save_draft(
            channel,
            DraftBody {
                message: OutgoingMessage {
                    content: "synthetic".into(),
                    attachments: vec![OutgoingAttachment {
                        filename: "proof.bin".into(),
                        bytes: vec![0, 255, 1],
                    }],
                    ..Default::default()
                },
                mentions: vec![session.account().user()],
            },
            UnixMicros(0),
            Some(0),
        )
        .await?;
    let first = Uuid::from_u128(1);
    let second = Uuid::from_u128(2);
    queue
        .enqueue_draft(channel, first, revision, UnixMicros(2))
        .await?;
    dependent_actions(&queue, channel, first, second).await?;
    let mut image = queue.subscribe().await?;
    loop {
        let value = image.borrow_and_update().clone();
        if value
            .operations
            .iter()
            .any(|op| matches!(op.state, OperationState::Unknown | OperationState::Failed))
        {
            return Err("proof mutation was not confirmed".into());
        }
        if value.operations.len() == 4
            && value
                .operations
                .iter()
                .all(|op| op.state == OperationState::Sent)
        {
            break;
        }
        image.changed().await?;
    }
    let draft = queue.draft(channel).await?.ok_or("draft missing")?;
    assert!(draft.revision > revision && draft.body.message.attachments.is_empty());
    println!("Sent 4; dependencies ordered; captured draft cleared");
    Ok(())
}

async fn dependent_actions(
    queue: &papo_sdk::OutgoingService,
    channel: Uuid,
    first: Uuid,
    second: Uuid,
) -> Result<(), Box<dyn Error>> {
    queue
        .enqueue(
            channel,
            second,
            OperationKind::Message(OutgoingMessage {
                content: "synthetic".into(),
                reply_to: Some(MessageTarget::Local(first)),
                ..Default::default()
            }),
            UnixMicros(1),
        )
        .await?;
    queue
        .enqueue(
            channel,
            Uuid::from_u128(3),
            OperationKind::Edit {
                target: MessageTarget::Local(second),
                content: "synthetic-edit".into(),
                embeds: None,
            },
            UnixMicros(0),
        )
        .await?;
    queue
        .enqueue(
            channel,
            Uuid::from_u128(4),
            OperationKind::Delete {
                target: MessageTarget::Local(second),
            },
            UnixMicros(-1),
        )
        .await?;
    Ok(())
}
