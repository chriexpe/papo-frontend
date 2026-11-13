//! Advisory sequential/concurrent warm cache-hit measurements; deterministic safety tests own acceptance.

use papo_sdk::{
    Client, ClientConfig, ConnectionState, DataRoot, LoginRequest, MediaSource, RegisterRequest,
    Session, UnixMicros, Uuid, secret::test_store::ScriptedSecretStore,
};
use papo_test_server::{Config, EncodedMedia, TestServer};
use std::{collections::BTreeSet, error::Error, process::ExitCode, sync::Arc, time::Instant};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
const OBJECT_BYTES: usize = 16 * 1024 * 1024;
const REQUESTS: usize = 4;
const ROUNDS: usize = 8;

fn main() -> ExitCode {
    if run().is_ok() {
        ExitCode::SUCCESS
    } else {
        eprintln!("media cost fixture failed");
        ExitCode::FAILURE
    }
}

fn run() -> Result {
    let profile = tempfile::tempdir()?;
    let mut client = Client::open(
        DataRoot::new(profile.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig {
            insecure_loopback: true,
            ..ClientConfig::default()
        },
    )?;
    let result = client.block_on(exercise(&client));
    let stopped = client.shutdown();
    result??;
    let report = stopped?;
    assert!(report.reads_drained && report.shared.finished && report.state.finished);
    Ok(())
}

async fn sign_in(client: &Client, server: papo_sdk::ServerId, index: usize) -> Result<Session> {
    let auth = client.handle().auth(server)?;
    let username = format!("media-cost-{index}");
    let password: String = (0_u8..32)
        .map(|n| char::from(b'A' + n % 26))
        .chain(['!', '1'])
        .collect();
    auth.register(RegisterRequest {
        username: username.clone(),
        password: password.clone(),
    })
    .await?;
    let session = auth
        .login(LoginRequest { username, password }, UnixMicros(0))
        .await?;
    let mut connection = session.subscribe_connection();
    while connection.borrow_and_update().state != ConnectionState::Online {
        connection.changed().await?;
    }
    Ok(session)
}

async fn exercise(client: &Client) -> Result {
    let server = TestServer::start(Config::default()).await?;
    let result = measure(client, &server).await;
    server.shutdown().await?;
    result
}

async fn measure(client: &Client, server: &TestServer) -> Result {
    let saved = client
        .handle()
        .add_server(&server.base_url(), UnixMicros(0))
        .await?;
    let sessions = [
        sign_in(client, saved, 0).await?,
        sign_in(client, saved, 1).await?,
    ];
    let users = sessions
        .iter()
        .map(|session| session.account().user().to_string())
        .collect::<BTreeSet<_>>();
    let mut sources = Vec::new();
    for index in 0..REQUESTS {
        let id = Uuid::from_u128(100 + index as u128);
        let source = MediaSource::Attachment(id);
        server
            .set_encoded_media(EncodedMedia {
                path: format!("/attachments/{id}"),
                bytes: vec![u8::try_from(index)?; OBJECT_BYTES],
                users: users.clone(),
                etag: None,
                status: 200,
                content_range: None,
            })
            .await?;
        sessions[index % 2]
            .media()
            .fetch(source.clone(), false)
            .await?;
        sources.push(source);
    }
    for parallel in [false, true] {
        let started = Instant::now();
        for _ in 0..ROUNDS {
            if parallel {
                let mut tasks = tokio::task::JoinSet::new();
                for (index, source) in sources.iter().cloned().enumerate() {
                    let media = sessions[index % 2].media();
                    tasks.spawn(async move { media.cached(source).await });
                }
                while let Some(result) = tasks.join_next().await {
                    assert_eq!(result??.value.bytes.len(), OBJECT_BYTES);
                }
            } else {
                for (index, source) in sources.iter().cloned().enumerate() {
                    assert_eq!(
                        sessions[index % 2]
                            .media()
                            .cached(source)
                            .await?
                            .value
                            .bytes
                            .len(),
                        OBJECT_BYTES
                    );
                }
            }
        }
        println!(
            "{}",
            serde_json::json!({
                "schema_version": 1, "scenario": "warm-media-hits", "parallel": parallel,
                "accounts": 2, "requests_per_round": REQUESTS, "rounds": ROUNDS,
                "object_bytes": OBJECT_BYTES, "elapsed_us_advisory": started.elapsed().as_micros(),
                "scope": "Warm encoded file reads plus SHA-256, ref validation and LRU; no decoder or native rendering"
            })
        );
    }
    Ok(())
}
