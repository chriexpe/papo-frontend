//! Restart and commit-point tests for v2 resource revisions, including older embedded summaries.

use super::{AccountCache, CacheError, CacheResource, resource_fixtures::snapshot};
use crate::{
    Generation, Uuid,
    resources::{AdmittedMutation, SummaryBatch},
    storage_worker::WorkerError,
};
use rusqlite::params;
use std::sync::Arc;

const GENERATION: Generation = Generation(4);
const CHANNEL: Uuid = Uuid::from_u128(5);
const ROLE: Uuid = Uuid::from_u128(6);
const PEER: Uuid = Uuid::from_u128(7);

async fn write_mutation(
    cache: &AccountCache,
    last: &crate::ResourceSnapshot,
    change: AdmittedMutation,
) -> crate::ResourceSnapshot {
    let mut next = change.apply(last, cache.account.user()).unwrap();
    next.version = last.version + 1;
    let changed = next.changed_groups(last);
    cache.update(&next, &changed).await.unwrap();
    next
}

#[test]
fn role_rename_deletion_and_summary_absence_survive_restart() {
    super::history_tests::run(|caches, _| async move {
        let account = super::history_tests::account();
        let cache = caches.open(account).await.unwrap();
        cache.replace(&snapshot()).await.unwrap();
        let old = cache.load(GENERATION).await.unwrap();
        let mut role = old.roles.get(ROLE).unwrap().clone();
        role.name = Some("renamed".into());
        let renamed = write_mutation(
            &cache,
            &old,
            AdmittedMutation {
                roles: Some(vec![role]),
                ..Default::default()
            },
        )
        .await;
        let hydrated = cache.load(GENERATION).await.unwrap();
        assert_eq!(
            hydrated.roles.get(ROLE).unwrap().name,
            renamed.roles.get(ROLE).unwrap().name
        );
        assert_eq!(hydrated.users.get(PEER).unwrap().roles, Some(vec![ROLE]));

        let deleted = write_mutation(
            &cache,
            &renamed,
            AdmittedMutation {
                roles: Some(Vec::new()),
                ..Default::default()
            },
        )
        .await;
        let hydrated = cache.load(GENERATION).await.unwrap();
        assert!(hydrated.roles.is_empty());
        assert_eq!(hydrated.users.get(PEER).unwrap().roles, Some(Vec::new()));
        let absent = write_mutation(
            &cache,
            &deleted,
            AdmittedMutation {
                summaries: vec![SummaryBatch {
                    present: Vec::new(),
                    absent: vec![PEER],
                }],
                ..Default::default()
            },
        )
        .await;
        caches.close(account).await.unwrap();
        let reopened = caches.open(account).await.unwrap();
        let current = reopened.load(Generation(5)).await.unwrap();
        assert_eq!(current.members, absent.members);
        assert!(current.users.get(PEER).is_none());
        assert!(current.profiles.get(PEER).is_none());
        assert!(current.roles.is_empty());
        assert_eq!(current.blocks, absent.blocks);
        assert_eq!(current.dms, absent.dms);
        assert_eq!(current.channels, old.channels);
        assert!(!current.channel_census_complete);
        caches.close(account).await.unwrap();
    });
}

#[test]
fn legacy_labels_and_inadmissible_channels_do_not_taint_members_or_history() {
    super::history_tests::run(|caches, _| async move {
        let account = super::history_tests::account();
        let cache = caches.open(account).await.unwrap();
        cache.replace(&snapshot()).await.unwrap();
        let set = cache
            .history_authorize(*CHANNEL.as_bytes(), GENERATION, false)
            .await
            .unwrap();
        // Materialize old v2 rows directly, including obsolete denormalized labels and
        // an overwrite group a newer admission rejects. Their SQLite ownership is valid.
        cache.writer.cache_write(|tx| {
            let member = serde_json::json!({"id": PEER, "nickname":"roster", "roles":[{"id":ROLE,"name":"obsolete"}]});
            tx.execute("UPDATE members SET payload=? WHERE id=?", params![serde_json::to_vec(&member).unwrap(),PEER.as_bytes()])?;
            let dm = serde_json::json!({"id": Uuid::from_u128(8),"user":{"id":PEER,"nickname":"stale","roles":[{"id":ROLE,"name":"obsolete"}]},"created_at":"","unread_count":0});
            tx.execute("UPDATE dms SET payload=?", [serde_json::to_vec(&dm).unwrap()])?;
            let channel = serde_json::json!({"id":CHANNEL,"type":"text","permissions":null});
            tx.execute("UPDATE channels SET payload=?", [serde_json::to_vec(&channel).unwrap()])?;
            Ok::<_, WorkerError>(())
        }).await.unwrap();
        caches.close(account).await.unwrap();
        let cache = caches.open(account).await.unwrap();
        let current = cache.load(Generation(5)).await.unwrap();
        assert!(current.channels.is_empty());
        assert!(!current.channel_census_complete);
        assert_eq!(
            current.roles.get(ROLE).unwrap().name.as_deref(),
            Some("original")
        );
        assert_eq!(
            current.users.get(PEER).unwrap().nickname.as_deref(),
            Some("roster")
        );
        assert_eq!(
            current.profiles.get(PEER).unwrap().description.as_deref(),
            Some("detail")
        );
        let set_on_disk: i64 = cache.readers.query(cache.path.clone(), |tx| {
            tx.query_row("SELECT current_set FROM history_channels WHERE channel_id=? AND verified_generation IS NULL", [CHANNEL.as_bytes()], |row| row.get(0))
        }).await.unwrap();
        assert_eq!(set_on_disk, set);
        assert!(
            cache
                .history_validate(*CHANNEL.as_bytes(), set, Generation(5))
                .await
                .is_err()
        );
        caches.close(account).await.unwrap();
    });
}

#[test]
fn failed_pointer_switch_keeps_the_previous_roles_channels_and_version() {
    let initialize = Arc::new(|connection: &mut rusqlite::Connection| {
        connection.execute_batch(
            "CREATE TRIGGER fail_switch BEFORE UPDATE OF channels_revision ON cache_metadata
            WHEN OLD.channels_revision>0 BEGIN SELECT RAISE(ABORT, 'fixture'); END",
        )?;
        Ok(())
    });
    super::history_tests::run_with_initializer(initialize, |caches, _| async move {
        let account = super::history_tests::account();
        let cache = caches.open(account).await.unwrap();
        let initial = snapshot();
        cache.replace(&initial).await.unwrap();
        let empty = crate::ResourceSnapshot::empty(GENERATION);
        let result = cache
            .update_resources(&empty, &[CacheResource::Roles, CacheResource::Channels])
            .await;
        assert!(matches!(result, Err(CacheError::Storage(_))));
        let loaded = cache.load(GENERATION).await.unwrap();
        assert_eq!(loaded.roles, initial.roles);
        assert_eq!(loaded.channels, initial.channels);
        assert_eq!(loaded.version, initial.version);
        caches.close(account).await.unwrap();
    });
}

#[test]
fn cleanup_failure_reports_committed_revision_and_preserves_all_selected_groups() {
    let initialize = Arc::new(|connection: &mut rusqlite::Connection| {
        connection.execute_batch("CREATE TRIGGER fail_prune BEFORE DELETE ON roles BEGIN SELECT RAISE(ABORT, 'fixture'); END")?;
        Ok(())
    });
    super::history_tests::run_with_initializer(initialize, |caches, _| async move {
        let account = super::history_tests::account();
        let cache = caches.open(account).await.unwrap();
        let initial = snapshot();
        cache.replace(&initial).await.unwrap();
        let mut empty = crate::ResourceSnapshot::empty(GENERATION);
        empty.version = initial.version + 1;
        let result = cache
            .update_resources(&empty, &[CacheResource::Roles, CacheResource::Channels])
            .await;
        assert!(matches!(result, Ok(Some(CacheError::Storage(_)))));
        let installed = cache.load(GENERATION).await.unwrap();
        assert_eq!(installed.version, empty.version);
        assert!(installed.roles.is_empty());
        assert!(installed.channels.is_empty());
        assert_eq!(installed.members, initial.members);
        // The writer reports its first maintenance fault again when it drains.
        assert!(matches!(
            caches.close(account).await,
            Err(CacheError::Storage(_))
        ));
    });
}

#[test]
fn canceled_caller_after_pointer_switch_keeps_the_switched_revision() {
    let (entered, pruning) = tokio::sync::oneshot::channel();
    let (release, resume) = std::sync::mpsc::channel();
    let hook = std::sync::Mutex::new(Some((entered, resume)));
    let initialize = Arc::new(move |connection: &mut rusqlite::Connection| {
        if let Some((signal, resume)) = hook.lock().unwrap().take() {
            let mut signal = Some(signal);
            connection.update_hook(Some(
                move |action: rusqlite::hooks::Action, _: &str, table: &str, _: i64| {
                    if action == rusqlite::hooks::Action::SQLITE_DELETE
                        && table == "channels"
                        && let Some(signal) = signal.take()
                    {
                        signal.send(()).unwrap();
                        resume.recv().unwrap();
                    }
                },
            ))?;
        }
        Ok(())
    });
    super::history_tests::run_with_initializer(initialize, |caches, _| async move {
        let account = super::history_tests::account();
        let cache = caches.open(account).await.unwrap();
        let initial = snapshot();
        cache.replace(&initial).await.unwrap();
        let writer = Arc::clone(&cache);
        let task = tokio::spawn(async move {
            writer
                .update_resources(
                    &crate::ResourceSnapshot::empty(GENERATION),
                    &[CacheResource::Channels, CacheResource::Roles],
                )
                .await
        });
        pruning.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(cache.serial.try_lock().is_err());
        release.send(()).unwrap();
        caches.close(account).await.unwrap();
        let cache = caches.open(account).await.unwrap();
        let installed = cache.load(Generation(5)).await.unwrap();
        assert!(installed.channels.is_empty());
        assert!(installed.roles.is_empty());
        assert_eq!(installed.members, initial.members);
        caches.close(account).await.unwrap();
    });
}

#[test]
#[ignore = "explicit full-roster SQLite scale measurement"]
fn measure_full_roster_restaging() {
    use std::time::Instant;
    super::history_tests::run(|caches, _| async move {
        let account = super::history_tests::account();
        let cache = caches.open(account).await.unwrap();
        for size in [10_000, 50_000] {
            let original = super::resource_fixtures::roster(size, GENERATION);
            cache.replace(&original).await.unwrap();
            let id = Uuid::from_u128(size / 2);
            let mut changed = original.clone();
            let mut user = changed.users.get(id).unwrap().clone();
            user.nickname = Some("updated".into());
            changed.users.set(id, user);
            let started = Instant::now();
            cache
                .update(&changed, &[CacheResource::Members])
                .await
                .unwrap();
            let elapsed = started.elapsed();
            let rows: i64 = cache
                .readers
                .query(cache.path.clone(), |tx| {
                    tx.query_row("SELECT COUNT(*) FROM members", [], |row| row.get(0))
                })
                .await
                .unwrap();
            assert_eq!(u128::try_from(rows).unwrap(), size);
            assert_eq!(original.users.get(id).unwrap().nickname, None);
            eprintln!(
                "SQLite full-roster restage: {size} member rows, 0 assignment edges, {} ms including encode, switch, prune and checkpoint",
                elapsed.as_millis()
            );
        }
        caches.close(account).await.unwrap();
    });
}
