//! Checks externally observable account cache isolation, removal and read marks; storage mechanics stay in unit tests.

use std::{future::Future, sync::Arc, time::Duration};

use papo_api::{
    Uuid,
    gateway::{KnownEvent, PresenceMember, PresenceSyncEvent},
    wire::{DirectConversation, UserSummary},
};
use papo_sdk::{
    Client, ClientConfig, ClientError, DataRoot, ServerReadPoint, UnixMicros,
    secret::test_store::ScriptedSecretStore,
};
use papo_test_server::{Config, TestServer};
use rusqlite::{Connection, params};

use super::{
    fixture::{Result, login, register},
    reconcile::{online, resources},
};

struct CacheFixture {
    client: Client,
    root: DataRoot,
    _profile: tempfile::TempDir,
}

impl CacheFixture {
    fn new() -> Result<Self> {
        let profile = tempfile::tempdir()?;
        let root = DataRoot::new(profile.path());
        let client = Client::open(
            root.clone(),
            Arc::new(ScriptedSecretStore::default()),
            ClientConfig {
                insecure_loopback: true,
                ..ClientConfig::default()
            },
        )?;
        Ok(Self {
            client,
            root,
            _profile: profile,
        })
    }

    fn run<F: Future>(&self, future: F) -> Result<F::Output> {
        Ok(self
            .client
            .block_on(async { tokio::time::timeout(Duration::from_secs(30), future).await })??)
    }

    fn finish(mut self) -> Result {
        let report = self.client.shutdown()?;
        assert!(report.reads_drained && report.state.finished && report.shared.finished);
        Ok(())
    }
}

async fn cache_revision(path: std::path::PathBuf) -> Result<i64> {
    Ok(tokio::task::spawn_blocking(move || {
        Connection::open(path)?.query_row(
            "SELECT next_revision FROM cache_metadata WHERE singleton=1",
            [],
            |row| row.get(0),
        )
    })
    .await??)
}

#[test]
fn presence_sync_replaces_memory_without_writing_presence_to_the_account_cache() -> Result {
    let fixture = CacheFixture::new()?;
    fixture.run(async {
        let server = TestServer::start(Config::default()).await?;
        let handle = fixture.client.handle();
        let server_id = handle.add_server(&server.base_url(), UnixMicros(0)).await?;
        let auth = handle.auth(server_id)?;
        register(&auth, "presence-cache").await?;
        let session = login(&auth, "presence-cache").await?;
        online(&session).await?;
        let cache_path = fixture.root.account_cache(session.account());
        let revision = cache_revision(cache_path.clone()).await?;
        let mut snapshots = session.subscribe_resources();
        let synthetic = Uuid::from_u128(77).to_string();
        server.emit_event(KnownEvent::PresenceSync(Box::new(PresenceSyncEvent {
            members: vec![PresenceMember { user_id: synthetic.clone(), status: "idle".into(), ..PresenceMember::default() }],
        }))).await?;
        resources(&mut snapshots, |snapshot| snapshot.presence.len() == 1 && snapshot.presence[0].user_id == synthetic).await?;
        server.emit_event(KnownEvent::PresenceSync(Box::default())).await?;
        resources(&mut snapshots, |snapshot| snapshot.presence.is_empty()).await?;
        server.emit_event(KnownEvent::PresenceSync(Box::new(PresenceSyncEvent {
            members: vec![PresenceMember { user_id: synthetic.clone(), status: "idle".into(), ..PresenceMember::default() }],
        }))).await?;
        resources(&mut snapshots, |snapshot| snapshot.presence.iter().any(|member| member.user_id == synthetic)).await?;
        assert_eq!(cache_revision(cache_path).await?, revision);
        session.reconnect()?;
        let generation = online(&session).await?;
        let snapshot = resources(&mut snapshots, |snapshot| snapshot.generation == generation && snapshot.presence.iter().any(|member| member.user_id == session.account().user().to_string())).await?;
        assert!(snapshot.presence.iter().all(|member| member.user_id != synthetic));
        let cache = fixture.root.account_cache(session.account());
        let rows = tokio::task::spawn_blocking(move || -> rusqlite::Result<Vec<String>> {
            let connection = Connection::open(cache)?;
            let mut statement = connection.prepare("SELECT name FROM sqlite_schema WHERE type='table'")?;
            statement.query_map([], |row| row.get(0))?.collect()
        }).await??;
        assert!(rows.iter().all(|table| !["presence", "activity", "typing", "voice"].contains(&table.as_str())));
        server.shutdown().await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn account_resources_are_isolated_and_late_removed_response_cannot_recreate_cache() -> Result {
    let fixture = CacheFixture::new()?;
    fixture.run(async {
        let server = TestServer::start(Config::default()).await?;
        let handle = fixture.client.handle();
        let server_id = handle.add_server(&server.base_url(), UnixMicros(0)).await?;
        let auth = handle.auth(server_id)?;
        register(&auth, "cache-first").await?;
        register(&auth, "cache-second").await?;
        let first = login(&auth, "cache-first").await?;
        let second = login(&handle.auth(server_id)?, "cache-second").await?;
        online(&first).await?;
        online(&second).await?;
        let blocked = UserSummary {
            id: Some(Uuid::from_u128(88).to_string()),
            username: Some("blocked-fixture".into()),
            ..UserSummary::default()
        };
        server
            .set_blocks(&first.account().user().to_string(), vec![blocked.clone()])
            .await;
        let dm = DirectConversation {
            id: Uuid::from_u128(89).to_string(),
            ..DirectConversation::default()
        };
        server
            .set_direct_conversations(&first.account().user().to_string(), vec![dm])
            .await?;
        first.reconnect()?;
        online(&first).await?;
        assert_eq!(
            first.subscribe_resources().borrow().blocks[0].id,
            blocked.id
        );
        assert_eq!(first.subscribe_resources().borrow().dms.len(), 1);
        assert!(second.subscribe_resources().borrow().blocks.is_empty());
        assert!(second.subscribe_resources().borrow().dms.is_empty());
        let first_path = fixture.root.account_cache(first.account());
        let second_path = fixture.root.account_cache(second.account());
        assert_ne!(first_path, second_path);
        assert!(first_path.exists() && second_path.exists());
        let mut old_resources = first.subscribe_resources();
        let mut hold = server.hold_next_rest("GET", "/channels").await?;
        first.reconnect()?;
        hold.wait_started().await?;
        handle.remove_account(first.account()).await?;
        assert!(first.is_closed());
        assert!(!first_path.exists());
        hold.release();
        assert!(first.reconnect().is_err());
        assert!(matches!(
            first.profiles(&[]).await,
            Err(ClientError::Closed)
        ));
        while old_resources.changed().await.is_ok() {
            old_resources.borrow_and_update();
        }
        second.reconnect()?;
        online(&second).await?;
        assert!(!first_path.exists());
        assert!(second_path.exists());
        server.shutdown().await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}

#[test]
fn reconnect_refreshes_server_read_point_without_touching_local_pending_bytes() -> Result {
    let fixture = CacheFixture::new()?;
    fixture.run(async {
        let server = TestServer::start(Config::default()).await?;
        let channel = server.create_channel("read-mark-channel").await?;
        let channel_id = Uuid::parse_str(channel.id.as_deref().unwrap())?;
        let handle = fixture.client.handle();
        let server_id = handle.add_server(&server.base_url(), UnixMicros(0)).await?;
        let auth = handle.auth(server_id)?;
        register(&auth, "read-point-user").await?;
        let session = login(&auth, "read-point-user").await?;
        online(&session).await?;
        let account = session.account();
        let path = fixture.root.state();
        let pending = serde_json::to_vec(&papo_sdk::ReadPoint {
            created_at_us: 1_234_567,
            message_id: Uuid::from_u128(91),
        })?;
        let expected = pending.clone();
        tokio::task::spawn_blocking(move || -> rusqlite::Result<()> {
            let connection = Connection::open(path)?;
            connection.execute("INSERT INTO read_points(server_id,user_id,channel_id,local_point) VALUES(?2,?3,?4,?1) ON CONFLICT(server_id,user_id,channel_id) DO UPDATE SET local_point=excluded.local_point", params![pending, account.server().as_bytes(), account.user().as_bytes(), channel_id.as_bytes()])?;
            Ok(())
        }).await??;
        let message = Uuid::from_u128(90);
        let recorded_at = "2026-11-09T12:00:00.000001Z";
        server.set_read_mark(&account.user().to_string(), channel.id.as_deref().unwrap(), &message.to_string(), recorded_at).await?;
        let pending_upload = server.hold_next_rest("PUT", &format!("/channels/{channel_id}/read")).await?;
        session.reconnect()?;
        online(&session).await?;
        let path = fixture.root.state();
        let (local, uploaded) = tokio::task::spawn_blocking(move || -> rusqlite::Result<(Vec<u8>, Vec<u8>)> {
            Connection::open(path)?.query_row("SELECT local_point,uploaded_point FROM read_points WHERE server_id=?1 AND user_id=?2 AND channel_id=?3", params![account.server().as_bytes(), account.user().as_bytes(), channel_id.as_bytes()], |row| Ok((row.get(0)?, row.get(1)?)))
        }).await??;
        assert_eq!(local, expected);
        let uploaded: ServerReadPoint = serde_json::from_slice(&uploaded)?;
        assert_eq!(uploaded.message_id, message);
        assert_eq!(uploaded.recorded_at.as_deref(), Some(recorded_at));
        pending_upload.release();
        server.shutdown().await?;
        Ok::<_, Box<dyn std::error::Error>>(())
    })??;
    fixture.finish()
}
