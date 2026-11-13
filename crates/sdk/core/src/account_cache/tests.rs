//! Account-isolated cache persistence and recovery; session identity verification is tested above this layer.

use super::*;
use crate::{
    ServerAddress,
    storage_worker::{StorageWorker, WorkerConfig},
};
use papo_api::{
    Uuid,
    wire::{
        Channel, ChannelPermissionEntry, ChannelPermissions, DirectConversation, Role, RoleSummary,
        Server, UserProfile, UserSummary,
    },
};
use std::time::{Duration, Instant};

fn account(user: u128) -> AccountId {
    AccountId::new(
        ServerAddress::parse("https://example.invalid")
            .unwrap()
            .id(),
        Uuid::from_u128(user),
    )
}

fn factory(root: DataRoot, worker: &StorageWorker) -> AccountCaches {
    AccountCaches::new(
        root,
        Arc::new(worker.registrar()),
        Arc::new(ReadPool::default()),
    )
}

fn snapshot() -> ResourceSnapshot {
    let mut snapshot = ResourceSnapshot::empty(Generation(4));
    snapshot.version = 9;
    snapshot.server = Some(Arc::new(Server {
        id: Some(Uuid::from_u128(3).to_string()),
        name: Some("fixture".into()),
        icon_blob: Some("image".into()),
        ..Server::default()
    }));
    Arc::make_mut(&mut snapshot.channels).push(Channel {
        id: Some(Uuid::from_u128(5).to_string()),
        name: Some("fixture".into()),
        permissions: Some(vec![ChannelPermissionEntry {
            role_id: Some(Uuid::from_u128(6).to_string()),
            permissions: Some(ChannelPermissions {
                read_channel: Some(false),
                send_messages: Some(true),
                ..ChannelPermissions::default()
            }),
            ..ChannelPermissionEntry::default()
        }]),
        ..Channel::default()
    });
    Arc::make_mut(&mut snapshot.roles).push(Role {
        id: Some(Uuid::from_u128(6).to_string()),
        ..Role::default()
    });
    let summary = UserSummary {
        roles: Some(vec![RoleSummary {
            id: Some(Uuid::from_u128(6).to_string()),
            ..RoleSummary::default()
        }]),
        id: Some(Uuid::from_u128(7).to_string()),
        status: Some("online".into()),
        status_message: Some("live".into()),
        typing: Some("live".into()),
        status_updated_at: Some("live".into()),
        ..UserSummary::default()
    };
    Arc::make_mut(&mut snapshot.members).push(summary.clone());
    Arc::make_mut(&mut snapshot.blocks).push(summary.clone());
    Arc::make_mut(&mut snapshot.profiles).push(UserProfile {
        id: summary.id.clone(),
        status: summary.status.clone(),
        avatar_blob: Some("image".into()),
        ..UserProfile::default()
    });
    Arc::make_mut(&mut snapshot.dms).push(DirectConversation {
        id: Uuid::from_u128(8).to_string(),
        user: summary,
        ..DirectConversation::default()
    });
    snapshot
}

#[test]
fn cache_is_lazy_and_isolated_and_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let root = DataRoot::new(directory.path());
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let caches = factory(root.clone(), &worker);
    assert!(!root.account_cache(account(1)).exists());
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let first = caches.open(account(1)).await.unwrap();
        let duplicate = caches.open(account(1)).await.unwrap();
        assert!(Arc::ptr_eq(&first, &duplicate));
        first.replace(&snapshot()).await.unwrap();
        let other = caches.open(account(2)).await.unwrap();
        assert!(other.load(Generation(5)).await.unwrap().channels.is_empty());
        caches.close(account(1)).await.unwrap();
        assert!(matches!(
            first.load(Generation(5)).await,
            Err(CacheError::Closed)
        ));
        let reopened = caches.open(account(1)).await.unwrap();
        let loaded = reopened.load(Generation(6)).await.unwrap();
        assert_eq!(loaded.version, 9);
        assert_eq!(loaded.generation, Generation(6));
        assert_eq!(loaded.channels.len(), 1);
        caches.invalidate(account(1)).await.unwrap();
        let rebuilt = caches.open(account(1)).await.unwrap();
        assert!(
            rebuilt
                .load(Generation(7))
                .await
                .unwrap()
                .channels
                .is_empty()
        );
        assert!(other.load(Generation(7)).await.unwrap().channels.is_empty());
        caches.close(account(1)).await.unwrap();
        caches.close(account(2)).await.unwrap();
    });
    worker
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn persistence_strips_nested_live_state_and_image_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let caches = factory(DataRoot::new(directory.path()), &worker);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let cache = caches.open(account(1)).await.unwrap();
        cache.replace(&snapshot()).await.unwrap();
        let loaded = cache.load(Generation(4)).await.unwrap();
        assert!(loaded.server.unwrap().icon_blob.is_none());
        assert!(loaded.profiles[0].avatar_blob.is_none());
        assert!(loaded.profiles[0].status.is_none());
        for user in [&loaded.members[0], &loaded.blocks[0], &loaded.dms[0].user] {
            assert!(user.status.is_none());
            assert!(user.status_message.is_none());
            assert!(user.typing.is_none());
            assert!(user.status_updated_at.is_none());
        }
        caches.close(account(1)).await.unwrap();
    });
    worker
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn unknown_versions_corrupt_files_and_wrong_identity_rebuild_before_reads() {
    for scenario in [
        "version_zero",
        "version_new",
        "version_negative",
        "metadata_version",
        "history_marker",
        "corrupt",
        "identity",
        "server_identity",
    ] {
        let directory = tempfile::tempdir().unwrap();
        let root = DataRoot::new(directory.path());
        let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
        let caches = factory(root.clone(), &worker);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            let cache = caches.open(account(1)).await.unwrap();
            cache.replace(&snapshot()).await.unwrap();
            caches.close(account(1)).await.unwrap();
        });
        let path = root.account_cache(account(1));
        if scenario == "corrupt" {
            std::fs::write(&path, b"not a sqlite database").unwrap();
        } else {
            let connection = rusqlite::Connection::open(&path).unwrap();
            match scenario {
                "version_zero" => connection.pragma_update(None, "user_version", 0).unwrap(),
                "version_new" => connection.pragma_update(None, "user_version", 55).unwrap(),
                "version_negative" => connection.pragma_update(None, "user_version", -1).unwrap(),
                "metadata_version" => {
                    connection
                        .execute("UPDATE cache_metadata SET schema_version=3", [])
                        .unwrap();
                }
                "history_marker" => {
                    connection
                        .execute("UPDATE cache_metadata SET history_revision=-1", [])
                        .unwrap();
                }
                "server_identity" => {
                    connection
                        .execute("UPDATE cache_metadata SET server_id=zeroblob(16)", [])
                        .unwrap();
                }
                _ => {
                    connection
                        .execute(
                            "UPDATE cache_metadata SET user_id=?1",
                            [Uuid::from_u128(2).as_bytes().as_slice()],
                        )
                        .unwrap();
                }
            }
        }
        runtime.block_on(async {
            let cache = caches.open(account(1)).await.unwrap();
            assert!(
                cache.load(Generation(8)).await.unwrap().channels.is_empty(),
                "{scenario}"
            );
            caches.close(account(1)).await.unwrap();
        });
        worker
            .shutdown(Instant::now() + Duration::from_secs(3))
            .unwrap();
        worker.join().unwrap();
    }
}

#[test]
fn required_non_message_tables_round_trip_overwrites_and_blob_role_memberships() {
    let directory = tempfile::tempdir().unwrap();
    let mut worker = StorageWorker::shared(WorkerConfig::default()).unwrap();
    let caches = factory(DataRoot::new(directory.path()), &worker);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let cache = caches.open(account(1)).await.unwrap();
        let expected = snapshot();
        cache.replace(&expected).await.unwrap();
        let loaded = cache.load(Generation(4)).await.unwrap();
        assert_eq!(
            serde_json::to_value(loaded.channels.as_ref()).unwrap(),
            serde_json::to_value(expected.channels.as_ref()).unwrap()
        );
        assert_eq!(
            serde_json::to_value(loaded.roles.as_ref()).unwrap(),
            serde_json::to_value(expected.roles.as_ref()).unwrap()
        );
        let (tables, member_id, role_id): (Vec<String>, Vec<u8>, Vec<u8>) = cache
            .readers
            .query(cache.path.clone(), |transaction| {
                let tables = transaction
                    .prepare("SELECT name FROM sqlite_schema WHERE type='table' ORDER BY name")?
                    .query_map([], |row| row.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()?;
                let (member, role) = transaction.query_row(
                    "SELECT member_id,role_id FROM member_roles",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )?;
                Ok((tables, member, role))
            })
            .await
            .unwrap();
        assert_eq!(
            tables,
            [
                "blocks",
                "cache_metadata",
                "channels",
                "chunk_items",
                "chunks",
                "dms",
                "history_channels",
                "history_manifest",
                "history_sets",
                "member_roles",
                "members",
                "messages",
                "pins",
                "roles",
                "server_info",
                "sqlite_sequence",
                "users"
            ]
        );
        assert_eq!(member_id, Uuid::from_u128(7).as_bytes());
        assert_eq!(role_id, Uuid::from_u128(6).as_bytes());
        caches.close(account(1)).await.unwrap();
    });
    worker
        .shutdown(Instant::now() + Duration::from_secs(3))
        .unwrap();
    worker.join().unwrap();
}
