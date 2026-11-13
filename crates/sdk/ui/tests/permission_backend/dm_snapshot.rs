//! Actual REST list guarantees flow through `Session` reconciliation into `CapabilityService` for both block directions.
use papo_sdk::{
    Client, ClientConfig, ConnectionState, DataRoot, LoginRequest, RegisterRequest, Session,
    UnixMicros, Uuid, secret::test_store::ScriptedSecretStore,
};
use papo_sdk_ui::{CapabilityService, DirectMessageService};
use reqwest::{Client as Http, Method};
use serde_json::{Value, json};
use std::{error::Error, sync::Arc, time::Duration};
type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

#[test]
#[ignore = "requires isolated pinned backend and disposable PostgreSQL; run scripts/papo-backend-proof.py"]
fn reverse_blocks_and_hidden_dms_are_unavailable_after_actual_rest_reconciliation() -> Result {
    let profile = tempfile::tempdir()?;
    let mut client = Client::open(
        DataRoot::new(profile.path()),
        Arc::new(ScriptedSecretStore::default()),
        ClientConfig {
            insecure_loopback: true,
            ..ClientConfig::default()
        },
    )?;
    let result = client.block_on(async {
        let address = std::env::var("PAPO_BACKEND_PROOF_ADDRESS")?;
        let password = std::env::var("PAPO_PROOF_PASSWORD")?;
        let handle = client.handle();
        let server = handle.add_server(&address, UnixMicros(0)).await?;
        let auth = handle.auth(server)?;
        for username in ["dm-snapshot-a", "dm-snapshot-b"] {
            auth.register(RegisterRequest {
                username: username.into(),
                password: password.clone(),
            })
            .await?;
        }
        let session = auth
            .login(
                LoginRequest {
                    username: "dm-snapshot-a".into(),
                    password: password.clone(),
                },
                UnixMicros(0),
            )
            .await?;
        online(&session).await?;
        let own = Http::builder().cookie_store(true).build()?;
        let peer = Http::builder().cookie_store(true).build()?;
        login(&own, &address, "dm-snapshot-a", &password).await?;
        login(&peer, &address, "dm-snapshot-b", &password).await?;
        let peer_id = request(&peer, Method::GET, &format!("{address}/auth/whoami"), None)
            .await?
            .json::<Value>()
            .await
            .map_err(|_| "invalid loopback body")?["id"]
            .as_str()
            .ok_or("missing peer")?
            .to_owned();
        let dm: Value = request(
            &own,
            Method::POST,
            &format!("{address}/dms"),
            Some(json!({"user_id": peer_id})),
        )
        .await?
        .json()
        .await
        .map_err(|_| "invalid loopback body")?;
        let channel: Uuid = dm["id"].as_str().ok_or("missing DM")?.parse()?;
        let capabilities = CapabilityService::new(session.clone());
        refresh(&session).await?;
        assert!(
            capabilities
                .channel(channel)
                .map_err(|_| "initial DM capability unavailable")?
                .value
                .read
        );
        assert!(
            capabilities
                .channel(channel)
                .map_err(|_| "initial DM send unavailable")?
                .value
                .send
        );

        exercise_block_visibility(&own, &peer, &address, &session, &peer_id, channel).await?;
        Ok::<_, Box<dyn Error>>(())
    })?;
    let shutdown = client.shutdown()?;
    assert!(shutdown.reads_drained);
    result
}

async fn exercise_block_visibility(
    own: &Http,
    peer: &Http,
    address: &str,
    session: &Session,
    peer_id: &str,
    channel: Uuid,
) -> Result {
    let capabilities = CapabilityService::new(session.clone());
    // Reverse block: own /users/blocks is empty, but own /dms must exclude the conversation.
    let reverse_block = format!("{address}/users/{}/block", session.account().user());
    request(peer, Method::POST, &reverse_block, None).await?;
    assert_unavailable(own, address, session, channel).await?;
    assert_eq!(session.subscribe_resources().borrow().blocks.len(), 0);
    request(peer, Method::DELETE, &reverse_block, None).await?;
    // Unblocking does not reopen the DM: storage hides it for both participants on block.
    assert_unavailable(own, address, session, channel).await?;
    request(
        own,
        Method::POST,
        &format!("{address}/dms"),
        Some(json!({"user_id": peer_id})),
    )
    .await?;
    refresh(session).await?;
    assert!(
        capabilities
            .channel(channel)
            .map_err(|_| "DM unavailable after reverse unblock")?
            .value
            .send
    );

    let own_block = format!("{address}/users/{peer_id}/block");
    request(own, Method::POST, &own_block, None).await?;
    assert_unavailable(own, address, session, channel).await?;
    assert_eq!(session.subscribe_resources().borrow().blocks.len(), 1);
    request(own, Method::DELETE, &own_block, None).await?;
    assert_unavailable(own, address, session, channel).await?;
    request(
        own,
        Method::POST,
        &format!("{address}/dms"),
        Some(json!({"user_id": peer_id})),
    )
    .await?;
    refresh(session).await?;
    assert!(
        capabilities
            .channel(channel)
            .map_err(|_| "DM unavailable after own unblock")?
            .value
            .send
    );

    request(
        own,
        Method::DELETE,
        &format!("{address}/dms/{channel}"),
        None,
    )
    .await?;
    assert_unavailable(own, address, session, channel).await?;
    // GET one DM still succeeds when hidden, proving it cannot establish list visibility.
    request(own, Method::GET, &format!("{address}/dms/{channel}"), None).await?;
    Ok(())
}

async fn request(
    client: &Http,
    method: Method,
    url: &str,
    body: Option<Value>,
) -> Result<reqwest::Response> {
    let mut request = client.request(method, url);
    if let Some(body) = body {
        request = request.json(&body);
    }
    let response = request
        .send()
        .await
        .map_err(|_| "loopback transport failed")?;
    if !response.status().is_success() {
        return Err("loopback backend rejected request".into());
    }
    Ok(response)
}
async fn login(client: &Http, address: &str, username: &str, password: &str) -> Result {
    request(
        client,
        Method::POST,
        &format!("{address}/auth/login"),
        Some(json!({"username": username, "password": password})),
    )
    .await?;
    Ok(())
}
async fn online(session: &Session) -> Result {
    let mut connection = session.subscribe_connection();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let status = *connection.borrow_and_update();
            if status.state == ConnectionState::Online && status.generation == session.generation()
            {
                return Ok::<_, Box<dyn Error>>(());
            }
            connection.changed().await?;
        }
    })
    .await??;
    Ok(())
}
async fn refresh(session: &Session) -> Result {
    session.reconnect()?;
    online(session).await
}
async fn assert_unavailable(
    client: &Http,
    address: &str,
    session: &Session,
    channel: Uuid,
) -> Result {
    let list: Value = request(client, Method::GET, &format!("{address}/dms"), None)
        .await?
        .json()
        .await
        .map_err(|_| "invalid loopback body")?;
    assert_eq!(list["dms"].as_array().ok_or("missing DM list")?.len(), 0);
    refresh(session).await?;
    assert!(
        CapabilityService::new(session.clone())
            .channel(channel)
            .is_err()
    );
    assert_eq!(
        DirectMessageService::new(vec![session.clone()])
            .subscribe()
            .snapshot
            .len(),
        0
    );
    Ok(())
}
