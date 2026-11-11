//! FULL commit/reopen, owner isolation and generation fences for durable read points.

use super::*;
use crate::{
    AuthState, DataRoot, ServerAddress,
    secret::{CredentialGeneration, SecretKey},
    storage_worker::{StorageWorker, WorkerConfig},
};
use std::{
    sync::atomic::AtomicU64,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

struct Fixture {
    _root: tempfile::TempDir,
    worker: StorageWorker,
    registry: Registry,
    session: Arc<SessionInner>,
    runtime: tokio::runtime::Runtime,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("state.sqlite");
        let (worker, writer) = StorageWorker::state(
            path.clone(),
            |connection| {
                crate::state_schema::migrate(connection)
                    .map_err(|_| crate::storage_worker::WorkerError::Initialization)
            },
            WorkerConfig::default(),
        )
        .unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let address = ServerAddress::parse("https://example.invalid").unwrap();
        let account = AccountId::new(address.id(), Uuid::from_u128(1));
        runtime.block_on(writer.write(move|tx| {
            tx.execute("INSERT INTO servers VALUES(?1,'https://example.invalid',0,0,0)",[account.server().as_bytes()])?;
            for user in [account.user(),Uuid::from_u128(2)] {
                tx.execute("INSERT INTO accounts(server_id,user_id,position,created_at_us) VALUES(?1,?2,0,0)",params![account.server().as_bytes(),user.as_bytes()])?;
            }
            Ok::<_,StorageError>(())
        })).unwrap();
        let registry = Registry {
            writer,
            readers: Arc::new(crate::storage_worker::ReadPool::default()),
            path,
            lease: DataRoot::new(root.path().join("lease")).acquire().unwrap(),
        };
        let session = SessionInner::new(
            account,
            address,
            SecretKey::new(account, CredentialGeneration::new(1)),
            Arc::new(AtomicU64::new(1)),
            CancellationToken::new(),
            AuthState::Ready,
        );
        Self {
            _root: root,
            worker,
            registry,
            session,
            runtime,
        }
    }
    fn finish(mut self) {
        self.worker
            .shutdown(Instant::now() + Duration::from_secs(2))
            .unwrap();
        self.worker.join().unwrap();
    }
}
fn point(created_at_us: i64, id: u128) -> ReadPoint {
    ReadPoint {
        created_at_us,
        message_id: Uuid::from_u128(id),
    }
}
fn uploaded(point: ReadPoint) -> UploadedPoint {
    UploadedPoint {
        server: ServerReadPoint {
            message_id: point.message_id,
            recorded_at: Some("2099-01-01T00:00:00Z".into()),
        },
        created_at_us: Some(point.created_at_us),
        known_point: Some(point),
    }
}

#[test]
fn local_acknowledgement_is_full_committed_and_reopens_at_microsecond_precision() {
    let fixture = Fixture::new();
    let channel = Uuid::from_u128(10);
    let generation = fixture.session.epochs.current().generation;
    fixture
        .runtime
        .block_on(fixture.registry.advance_local_read(
            Arc::clone(&fixture.session),
            channel,
            generation,
            point(123_456, 10),
        ))
        .unwrap();
    let connection = rusqlite::Connection::open(&fixture.registry.path).unwrap();
    let bytes: Vec<u8> = connection
        .query_row("SELECT local_point FROM read_points", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<ReadPoint>(&bytes).unwrap(),
        point(123_456, 10)
    );
    drop(connection);
    fixture.finish();
}

#[test]
fn local_durable_point_never_moves_backwards_in_same_generation() {
    let fixture = Fixture::new();
    let channel = Uuid::from_u128(10);
    let generation = fixture.session.epochs.current().generation;
    fixture.runtime.block_on(async {
        for point in [point(10, 10), point(9, 100), point(10, 9)] {
            fixture
                .registry
                .advance_local_read(Arc::clone(&fixture.session), channel, generation, point)
                .await
                .unwrap();
        }
        assert_eq!(
            fixture
                .registry
                .load_read_points(fixture.session.account)
                .await
                .unwrap()[&channel]
                .local,
            Some(point(10, 10))
        );
    });
    fixture.finish();
}

#[test]
fn server_refresh_preserves_local_pending_and_does_not_order_by_ack_time() {
    let fixture = Fixture::new();
    let channel = Uuid::from_u128(10);
    let generation = fixture.session.epochs.current().generation;
    fixture.runtime.block_on(async {
        fixture
            .registry
            .advance_local_read(
                Arc::clone(&fixture.session),
                channel,
                generation,
                point(10, 10),
            )
            .await
            .unwrap();
        fixture
            .registry
            .save_uploaded_read(
                Arc::clone(&fixture.session),
                channel,
                generation,
                uploaded(point(5, 5)),
            )
            .await
            .unwrap();
        let points = fixture
            .registry
            .load_read_points(fixture.session.account)
            .await
            .unwrap();
        assert_eq!(
            (
                points[&channel].local,
                points[&channel].uploaded.as_ref().unwrap().point()
            ),
            (Some(point(10, 10)), Some(point(5, 5)))
        );
    });
    fixture.finish();
}

#[test]
fn other_account_never_reads_owner_pending_state() {
    let fixture = Fixture::new();
    let channel = Uuid::from_u128(10);
    let generation = fixture.session.epochs.current().generation;
    fixture.runtime.block_on(async {
        fixture
            .registry
            .advance_local_read(
                Arc::clone(&fixture.session),
                channel,
                generation,
                point(10, 10),
            )
            .await
            .unwrap();
        let other = AccountId::new(fixture.session.account.server(), Uuid::from_u128(2));
        assert!(
            fixture
                .registry
                .load_read_points(other)
                .await
                .unwrap()
                .is_empty()
        );
    });
    fixture.finish();
}

#[test]
fn superseded_generation_cannot_persist_local_or_uploaded_points() {
    let fixture = Fixture::new();
    let channel = Uuid::from_u128(10);
    let generation = fixture.session.epochs.current().generation;
    fixture.session.epochs.advance().unwrap();
    fixture.runtime.block_on(async {
        assert!(matches!(
            fixture
                .registry
                .advance_local_read(
                    Arc::clone(&fixture.session),
                    channel,
                    generation,
                    point(10, 10)
                )
                .await,
            Err(ClientError::Superseded)
        ));
        assert!(matches!(
            fixture
                .registry
                .save_uploaded_read(
                    Arc::clone(&fixture.session),
                    channel,
                    generation,
                    uploaded(point(10, 10))
                )
                .await,
            Err(ClientError::Superseded)
        ));
        assert!(
            fixture
                .registry
                .load_read_points(fixture.session.account)
                .await
                .unwrap()
                .is_empty()
        );
    });
    fixture.finish();
}

#[test]
fn cancelled_owner_cannot_persist_even_when_account_row_remains_visible() {
    let fixture = Fixture::new();
    let channel = Uuid::from_u128(10);
    let generation = fixture.session.epochs.current().generation;
    fixture.session.root.cancel();
    fixture.runtime.block_on(async {
        assert!(matches!(
            fixture
                .registry
                .advance_local_read(
                    Arc::clone(&fixture.session),
                    channel,
                    generation,
                    point(10, 10)
                )
                .await,
            Err(ClientError::Closed)
        ));
        assert!(
            fixture
                .registry
                .load_read_points(fixture.session.account)
                .await
                .unwrap()
                .is_empty()
        );
    });
    fixture.finish();
}

#[test]
fn session_5b_unresolved_server_bytes_decode_without_inventing_order() {
    let raw = ServerReadPoint {
        message_id: Uuid::from_u128(7),
        recorded_at: Some("2099-01-01T00:00:00Z".into()),
    };
    let restored: UploadedPoint =
        serde_json::from_slice(&serde_json::to_vec(&raw).unwrap()).unwrap();
    assert_eq!(restored.point(), None);
}

#[test]
fn unresolved_replacement_retains_previously_resolved_deleted_anchor_order() {
    let value = UploadedPoint {
        server: ServerReadPoint {
            message_id: Uuid::from_u128(9),
            recorded_at: Some("2099-01-01T00:00:00Z".into()),
        },
        created_at_us: None,
        known_point: Some(point(7, 7)),
    };
    let restored: UploadedPoint =
        serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(restored.point(), Some(point(7, 7)));
}
