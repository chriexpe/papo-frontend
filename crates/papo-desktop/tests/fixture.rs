//! Real core runtime integration against an independently owned loopback fixture.
//!
//! These tests exercise REST paging, persistent sockets, reconnect replay and
//! explicit transport shutdown. No external network, real account or disk
//! credential is involved. Wall-clock deadlines bound integration I/O only.

#[path = "support/fixture_backend.rs"]
mod fixture_backend;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use papo_core::api::net::{Command, Update, Wake};
use papo_core::api::ws::{Connection, Event};
use papo_core::cache::ClientDb;
use papo_core::notification::NotificationCoordinator;
use papo_core::runtime::ServerRuntime;
use papo_core::state::{Screen, TimelineStatus};
use papo_core::storage::{MemorySecretStore, Secret, SecretStore as _};

use fixture_backend::{ACCOUNT_ID, CHANNEL_ID, FixtureBackend, SESSION_TOKEN, fixture_messages};

/// Integration deadline, including the core's shared REST admission budget.
const DEADLINE: Duration = Duration::from_secs(20);

#[track_caller]
fn drive_until(runtime: &mut ServerRuntime, ready: impl Fn(&ServerRuntime) -> bool) {
    let deadline = Instant::now() + DEADLINE;
    while !ready(runtime) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        assert!(
            !remaining.is_zero(),
            "local runtime timed out: connection={:?}, screen={:?}, rows={}, freshness={:?}",
            runtime.store.connection,
            runtime.store.screen,
            runtime.store.messages_in(CHANNEL_ID).count(),
            runtime.store.timeline_status(CHANNEL_ID)
        );
        let _ = runtime.recv_timeout(remaining.min(Duration::from_millis(100)));
        assert!(
            runtime.store.error.is_none(),
            "synthetic fixture runtime error: {:?}",
            runtime.store.error
        );
    }
}

struct TempCache(PathBuf);

impl TempCache {
    fn open() -> Result<(Self, Arc<ClientDb>), Box<dyn std::error::Error>> {
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "papo-desktop-fixture-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&directory)?;
        let cache = Arc::new(ClientDb::open(Some(directory.join("fixture.db"))));
        assert!(cache.is_enabled());
        Ok((Self(directory), cache))
    }
}

impl Drop for TempCache {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn authenticated_runtime(
    backend: &FixtureBackend,
    cache: Arc<ClientDb>,
) -> Result<ServerRuntime, papo_core::storage::StorageError> {
    let secrets = Arc::new(MemorySecretStore::default());
    secrets.store(
        &papo_core::server_key(backend.url()),
        Secret::SessionToken,
        SESSION_TOKEN,
    )?;
    let notification = Arc::new(NotificationCoordinator::new(Arc::clone(&cache), None));
    let mut runtime = ServerRuntime::open(
        backend.url().into(),
        Wake::noop(),
        secrets,
        cache,
        notification,
    );
    drive_until(&mut runtime, |runtime| {
        runtime.store.me == ACCOUNT_ID
            && runtime.store.connection == Connection::Online
            && runtime
                .store
                .channels
                .iter()
                .any(|channel| channel.id == CHANNEL_ID)
    });
    assert_eq!(runtime.store.screen, Screen::Chat);
    assert!(backend.wait_for(DEADLINE, |state| state.connections == 1));
    Ok(runtime)
}

fn load_older(runtime: &mut ServerRuntime) -> Option<()> {
    let (since, last_id) = runtime.store.begin_load_older(CHANNEL_ID)?;
    let ticket = runtime.store.history_ticket(CHANNEL_ID)?;
    runtime.net.send(Command::LoadOlderMessages {
        ticket,
        since,
        last_id,
    });
    drive_until(runtime, |runtime| !runtime.store.loading_older(CHANNEL_ID));
    Some(())
}

#[expect(
    clippy::expect_used,
    reason = "A missing synthetic replay is a failed integration test."
)]
fn consume_replay(runtime: &mut ServerRuntime, count: usize) {
    let deadline = Instant::now() + DEADLINE;
    let mut remaining = count;
    while remaining > 0 {
        let update = runtime
            .net
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("replay must reach runtime");
        if matches!(&update, Update::Event(event) if matches!(event.as_ref(), Event::Message(_))) {
            remaining -= 1;
        }
        runtime.process_update(update);
        assert!(runtime.store.error.is_none());
    }
}

#[test]
fn real_runtime_pages_reconnects_replays_and_joins_its_transport() {
    assert_eq!(fixture_messages(0..10_000).unwrap().len(), 10_000);
    let mut backend = FixtureBackend::start(10_000, 0).unwrap();
    let (_directory, cache) = TempCache::open().unwrap();
    let mut runtime = authenticated_runtime(&backend, cache).unwrap();
    runtime.store.selected_channel = CHANNEL_ID.into();
    let ticket = runtime.store.mark_loading(CHANNEL_ID);
    runtime.net.send(Command::LoadMessages { ticket });
    drive_until(&mut runtime, |runtime| {
        runtime.store.timeline_status(CHANNEL_ID) == TimelineStatus::Fresh
    });
    assert_eq!(runtime.store.messages_in(CHANNEL_ID).count(), 100);
    assert_eq!(
        runtime.store.messages_in(CHANNEL_ID).next().unwrap().id,
        "m00009900"
    );
    assert_eq!(
        runtime.store.newest_loaded(CHANNEL_ID).as_deref(),
        Some("m00009999")
    );

    load_older(&mut runtime).unwrap();
    assert_eq!(runtime.store.messages_in(CHANNEL_ID).count(), 200);
    assert_eq!(
        runtime.store.messages_in(CHANNEL_ID).next().unwrap().id,
        "m00009800"
    );

    while runtime.store.messages_in(CHANNEL_ID).count() < 2_000 {
        load_older(&mut runtime).unwrap();
    }
    let generation = runtime.store.sync_generation();
    let refreshes = runtime.diagnostics().reconnect_refreshes;
    assert!(backend.disconnect());
    drive_until(&mut runtime, |runtime| {
        runtime.store.sync_generation() > generation
    });
    runtime.net.send(Command::ProbeConnection);
    drive_until(&mut runtime, |runtime| {
        runtime.store.connection == Connection::Online
    });
    assert!(backend.wait_for(DEADLINE, |state| state.connections >= 2));
    assert_eq!(runtime.store.sync_generation(), generation + 1);
    assert_eq!(runtime.diagnostics().reconnect_refreshes, refreshes + 1);

    let ticket = runtime.store.mark_loading(CHANNEL_ID);
    runtime.net.send(Command::LoadMessages { ticket });
    drive_until(&mut runtime, |runtime| {
        runtime.store.timeline_status(CHANNEL_ID) == TimelineStatus::Fresh
    });
    assert!(backend.replay(2_000));
    consume_replay(&mut runtime, 2_000);
    drive_until(&mut runtime, |runtime| {
        runtime.store.timeline_status(CHANNEL_ID) == TimelineStatus::Fresh
            && runtime.store.messages_in(CHANNEL_ID).count() == 2_000
    });
    let ids: HashSet<_> = runtime
        .store
        .messages_in(CHANNEL_ID)
        .map(|message| &message.id)
        .collect();
    assert_eq!(ids.len(), 2_000);
    assert_eq!(
        runtime.store.message("m00009999").unwrap().content,
        "synthetic-replay-2-9999"
    );
    assert_eq!(
        runtime.store.messages_in(CHANNEL_ID).next().unwrap().id,
        "m00008000"
    );
    drive_until(&mut runtime, |runtime| {
        let diagnostics = runtime.diagnostics();
        diagnostics.net.scheduler.queued == 0 && diagnostics.net.scheduler.running == 0
    });
    let stats = backend.stats();
    assert_eq!(stats.emitted_events, 2_000);
    assert_eq!(stats.http_errors, 0);
    assert_eq!(stats.failures, 0);
    assert!(stats.history_requests >= 3);

    runtime.net.shutdown();
    assert!(backend.wait_for(DEADLINE, |state| state.active_sockets == 0));
    assert_eq!(backend.stats().active_sockets, 0);
    runtime.cache.flush();
    drop(runtime);
    assert!(backend.shutdown());
}
