//! Actual pinned Go operations on disposable PostgreSQL; run only through the proof harness.
#![cfg(feature = "test-support")]

#[path = "session13_proof/administration.rs"]
mod administration;
#[path = "session13_proof/audit.rs"]
mod audit;
#[path = "session13_proof/interactions.rs"]
mod interactions;
#[path = "session13_proof/personal.rs"]
mod personal;
#[path = "session13_proof/security.rs"]
mod security;
#[path = "session13_proof/webhooks.rs"]
mod webhooks;

use papo_sdk::{
    Client, ClientConfig, ConnectionState, DataRoot, LoginRequest, RegisterRequest, Session,
    UnixMicros, Uuid, secret::test_store::ScriptedSecretStore,
};
use std::{sync::Arc, time::Duration};
// Complete generated synthetic 1x1 PNG, with valid chunk CRCs.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
#[ignore = "requires isolated pinned Go/PostgreSQL; run scripts/papo-backend-proof.py"]
fn sdk_operations_follow_actual_pinned_backend_authorization_and_mutations() -> Result {
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
        tokio::time::timeout(Duration::from_secs(90), async {
            let server = client
                .handle()
                .add_server(&address, UnixMicros(0))
                .await?
                .id;
            let owner = login(&client, server, &password, "cursorproof", false).await?;
            let member = login(&client, server, &password, "operationsproof", true).await?;
            let other = login(&client, server, &password, "operationsthird", true).await?;
            let channel = administration::prove(&owner, &member).await?;
            personal::prove(&member, &other, &owner, channel).await?;
            webhooks::prove(&owner, &member, channel).await?;
            audit::prove(&owner, &member, &address).await?;
            security::prove(&owner, &member, &other).await?;
            Ok::<_, Box<dyn std::error::Error>>(())
        })
        .await?
    })??;
    let report = client.shutdown()?;
    assert!(report.state.finished && report.shared.finished);
    Ok(())
}

async fn login(
    client: &Client,
    server: papo_sdk::ServerId,
    password: &str,
    username: &str,
    register: bool,
) -> Result<Session> {
    let handle = client.handle();
    let auth = handle.auth(server)?;
    if register {
        auth.register(RegisterRequest {
            username: username.into(),
            password: password.into(),
        })
        .await?;
    }
    let session = auth
        .login(
            LoginRequest {
                username: username.into(),
                password: password.into(),
            },
            UnixMicros(0),
        )
        .await?;
    let mut connection = session.subscribe_connection();
    loop {
        let status = *connection.borrow_and_update();
        if status.generation == session.generation() && status.state == ConnectionState::Online {
            return Ok(session);
        }
        connection.changed().await?;
    }
}

fn id(value: Option<&str>) -> Result<Uuid> {
    Ok(value.ok_or("missing authoritative identity")?.parse()?)
}

async fn wait_sent(session: &Session, local: Uuid) -> Result<papo_sdk::Operation> {
    let mut outgoing = session.outgoing().subscribe().await?;
    loop {
        let snapshot = outgoing.borrow_and_update().clone();
        if let Some(operation) = snapshot
            .operations
            .iter()
            .find(|operation| operation.id == local)
        {
            if operation.state == papo_sdk::OperationState::Sent {
                return Ok(operation.as_ref().clone());
            }
            assert!(!matches!(
                operation.state,
                papo_sdk::OperationState::Failed | papo_sdk::OperationState::Unknown
            ));
        }
        outgoing.changed().await?;
    }
}

fn is_problem<T>(
    result: &std::result::Result<T, papo_sdk::OperationError>,
    status: u16,
    kind: &papo_api::ProblemKind,
) -> bool {
    matches!(
        result,
        Err(papo_sdk::OperationError::Session(
            papo_sdk::ClientError::Api(papo_api::Error::Problem {
                status: actual,
                problem,
            })
        )) if *actual == status && problem.kind() == *kind
    )
}

fn is_status<T>(result: &std::result::Result<T, papo_sdk::OperationError>, expected: u16) -> bool {
    matches!(
        result,
        Err(papo_sdk::OperationError::Session(
            papo_sdk::ClientError::Api(papo_api::Error::Problem { status, .. })
        )) if *status == expected
    )
}
