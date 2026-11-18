//! Admission transitions over a real account cache: the seal holds across stalled, cancelled or
//! failed durable revokes, unrelated channels keep progressing, and a regrant never reuses a
//! denied cache set. Deterministic seeded sequences replace wall-clock or random scheduling.

use super::{Admission, Transition};
use crate::{
    AccountId, AuthState, Client, ClientConfig, DataRoot, Generation, HistoryClosedReason,
    HistoryError, ResourceSnapshot, ServerAddress,
    reconcile::ResourceConnection,
    secret::{CredentialGeneration, SecretKey, test_store::ScriptedSecretStore},
    session::SessionInner,
};
use papo_api::{
    RestClient, Uuid,
    wire::{
        ChannelPermissionEntry, ChannelPermissions, DirectConversation, RoleSummary, Server,
        UserSummary,
    },
};
use std::{
    future::Future,
    sync::{Arc, atomic::AtomicU64},
    task::Poll,
};
use tokio_util::sync::CancellationToken;

pub(super) const ROLE: &str = "30000000-0000-4000-8000-000000000001";
pub(super) const OPEN: Uuid = Uuid::from_u128(8);
pub(super) const RESTRICTED: Uuid = Uuid::from_u128(7);
pub(super) const DM: Uuid = Uuid::from_u128(9);
pub(super) const PEER: Uuid = Uuid::from_u128(10);

#[derive(Clone, Copy, Debug)]
pub(super) enum Evidence {
    Allowed,
    Denied,
    Unknown,
}

/// The account (nil user) holds `ROLE`; `RESTRICTED` follows `evidence`, `OPEN` is open.
pub(super) fn snapshot(generation: Generation, evidence: Evidence) -> ResourceSnapshot {
    let mut snapshot = ResourceSnapshot::empty(generation);
    snapshot.server = Some(Arc::new(Server {
        id: Some(Uuid::from_u128(2).to_string()),
        owner_id: Some(Uuid::from_u128(1).to_string()),
        ..Server::default()
    }));
    snapshot.members = Arc::new(vec![UserSummary {
        id: Some(Uuid::nil().to_string()),
        roles: Some(vec![RoleSummary {
            id: Some(ROLE.into()),
            ..RoleSummary::default()
        }]),
        ..UserSummary::default()
    }]);
    let restricted = crate::Channel {
        id: Some(RESTRICTED.to_string()),
        r#type: Some("text".into()),
        permissions: match evidence {
            Evidence::Unknown => None,
            Evidence::Allowed | Evidence::Denied => Some(vec![ChannelPermissionEntry {
                role_id: Some(ROLE.into()),
                permissions: Some(ChannelPermissions {
                    read_channel: Some(matches!(evidence, Evidence::Allowed)),
                    ..ChannelPermissions::default()
                }),
                ..ChannelPermissionEntry::default()
            }]),
        },
        ..crate::Channel::default()
    };
    let open = crate::Channel {
        id: Some(OPEN.to_string()),
        r#type: Some("text".into()),
        permissions: Some(Vec::new()),
        ..crate::Channel::default()
    };
    snapshot.channels = Arc::new(vec![restricted, open]);
    snapshot
}

fn with_dm(mut snapshot: ResourceSnapshot, blocked: bool) -> ResourceSnapshot {
    snapshot.dms = Arc::new(vec![DirectConversation {
        id: DM.to_string(),
        user: UserSummary {
            id: Some(PEER.to_string()),
            ..UserSummary::default()
        },
        created_at: "2026-01-01T00:00:00Z".into(),
        last_message: None,
        last_read_message: None,
        last_read_at: None,
        unread_count: 0,
    }]);
    if blocked {
        snapshot.blocks = Arc::new(vec![UserSummary {
            id: Some(PEER.to_string()),
            ..UserSummary::default()
        }]);
    }
    snapshot
}

pub(super) struct Harness {
    pub client: Client,
    pub session: Arc<SessionInner>,
    pub connection: ResourceConnection,
    pub generation: Generation,
}

impl Harness {
    pub fn open(directory: &tempfile::TempDir) -> Self {
        let client = Client::open(
            DataRoot::new(directory.path()),
            Arc::new(ScriptedSecretStore::default()),
            ClientConfig::default(),
        )
        .unwrap();
        let address = ServerAddress::parse("https://example.invalid").unwrap();
        let account = AccountId::new(address.id(), Uuid::nil());
        let session = SessionInner::new(
            account,
            address.clone(),
            SecretKey::new(account, CredentialGeneration::new(1)),
            Arc::new(AtomicU64::new(1)),
            CancellationToken::new(),
            AuthState::Ready,
        );
        let generation = session.epochs.current().generation;
        let cache = client
            .block_on(client.handle().inner.caches.open(account))
            .unwrap()
            .unwrap();
        let connection = ResourceConnection {
            client: client.handle(),
            rest: RestClient::new(address.as_str()).unwrap(),
            cache,
        };
        Self {
            client,
            session,
            connection,
            generation,
        }
    }

    /// One producer's accepted transition: seal, write resources, grant.
    pub async fn accept(&self, snapshot: &ResourceSnapshot) -> Result<bool, crate::ClientError> {
        let access = Transition::begin(&self.session, &self.connection, snapshot).await?;
        self.connection.cache.replace(snapshot).await?;
        access.finish(&self.session, &self.connection).await
    }

    pub async fn set(&self, channel: Uuid) -> i64 {
        self.connection
            .cache
            .history_authorize(*channel.as_bytes(), self.generation, false)
            .await
            .unwrap()
    }

    pub async fn admission(&self, channel: Uuid) -> Option<Admission> {
        self.session.history.admission.read().await.get(channel)
    }

    pub fn finish(mut self) {
        self.client.block_on(self.session.stop()).unwrap();
        self.client.shutdown().unwrap();
    }
}

pub(super) fn poll_once<F: Future>(future: &mut std::pin::Pin<Box<F>>) -> Poll<F::Output> {
    let waker = std::task::Waker::noop();
    future
        .as_mut()
        .poll(&mut std::task::Context::from_waker(waker))
}

#[test]
fn stalled_revoke_keeps_the_seal_without_holding_unrelated_channels_behind_the_map_lock() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    let generation = harness.generation;
    harness
        .client
        .block_on(async {
            harness
                .accept(&snapshot(generation, Evidence::Allowed))
                .await
                .unwrap();
            let revoked = harness
                .session
                .history
                .verified_channel(RESTRICTED)
                .await
                .unwrap();
            let cache = Arc::clone(&harness.connection.cache);
            let (entered, waiting) = tokio::sync::oneshot::channel();
            let (release, blocked) = std::sync::mpsc::channel::<()>();
            let stall = tokio::spawn(async move {
                cache
                    .history_transaction(generation, move |_| {
                        entered.send(()).unwrap();
                        blocked.recv().unwrap();
                        Ok(())
                    })
                    .await
            });
            waiting.await.unwrap();
            let denied = snapshot(generation, Evidence::Denied);
            let mut seal = Box::pin(Transition::begin(
                &harness.session,
                &harness.connection,
                &denied,
            ));
            assert!(poll_once(&mut seal).is_pending());

            // The seal is in place before the durable revoke finishes...
            assert!(matches!(
                revoked.check(),
                Err(HistoryError::Closed(HistoryClosedReason::PermissionDenied))
            ));
            assert!(matches!(
                harness.session.history.verified_channel(RESTRICTED).await,
                Err(HistoryError::Closed(HistoryClosedReason::PermissionDenied))
            ));
            // ...and an unrelated allowed channel opens without waiting for storage (B-F4).
            assert!(harness.session.history.channels.try_lock().is_ok());
            let mut unrelated = Box::pin(harness.session.history.verified_channel(OPEN));
            assert!(matches!(poll_once(&mut unrelated), Poll::Ready(Ok(_))));

            release.send(()).unwrap();
            stall.await.unwrap().unwrap();
            seal.await
                .unwrap()
                .finish(&harness.session, &harness.connection)
                .await
                .unwrap();
            assert_eq!(
                harness.admission(RESTRICTED).await,
                Some(Admission::Denied {
                    reason: HistoryClosedReason::PermissionDenied,
                    durable: true
                })
            );
        })
        .unwrap();
    harness.finish();
}

#[test]
fn unknown_quarantines_the_same_incarnation_and_never_reopens_a_denial() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    let generation = harness.generation;
    harness
        .client
        .block_on(async {
            harness
                .accept(&snapshot(generation, Evidence::Allowed))
                .await
                .unwrap();
            let set = harness.set(RESTRICTED).await;
            let channel = harness
                .session
                .history
                .verified_channel(RESTRICTED)
                .await
                .unwrap();
            harness
                .accept(&snapshot(generation, Evidence::Unknown))
                .await
                .unwrap();
            assert!(matches!(channel.check(), Err(HistoryError::AccessUnknown)));
            assert!(matches!(
                harness.session.history.verified_channel(RESTRICTED).await,
                Err(HistoryError::AccessUnknown)
            ));
            assert!(harness.session.history.verified_channel(OPEN).await.is_ok());
            assert_eq!(
                harness
                    .session
                    .reading
                    .positions
                    .check_current_channel(RESTRICTED, generation),
                Some(crate::reading::positions::ChannelFence::Unknown)
            );

            harness
                .accept(&snapshot(generation, Evidence::Allowed))
                .await
                .unwrap();
            let resumed = harness
                .session
                .history
                .verified_channel(RESTRICTED)
                .await
                .unwrap();
            assert!(Arc::ptr_eq(&channel, &resumed));
            assert!(resumed.check().is_ok());
            assert_eq!(harness.set(RESTRICTED).await, set);
            assert_eq!(
                harness
                    .session
                    .reading
                    .positions
                    .check_current_channel(RESTRICTED, generation),
                None
            );

            // Denied, then Unknown: still denied, never quarantined back into reach.
            harness
                .accept(&snapshot(generation, Evidence::Denied))
                .await
                .unwrap();
            harness
                .accept(&snapshot(generation, Evidence::Unknown))
                .await
                .unwrap();
            assert!(matches!(
                harness.session.history.verified_channel(RESTRICTED).await,
                Err(HistoryError::Closed(HistoryClosedReason::PermissionDenied))
            ));
        })
        .unwrap();
    harness.finish();
}

#[test]
fn dm_block_denies_with_a_durable_marker_and_unblocking_regrants_a_fresh_set() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    let generation = harness.generation;
    harness
        .client
        .block_on(async {
            let base = snapshot(generation, Evidence::Allowed);
            harness.accept(&with_dm(base.clone(), false)).await.unwrap();
            let old_set = harness.set(DM).await;
            let old = harness.session.history.verified_channel(DM).await.unwrap();
            harness.accept(&with_dm(base.clone(), true)).await.unwrap();
            assert!(matches!(
                old.check(),
                Err(HistoryError::Closed(HistoryClosedReason::PermissionDenied))
            ));
            // A reverse block removes the DM from the pinned list.
            harness.accept(&base).await.unwrap();
            assert!(matches!(
                harness.session.history.verified_channel(DM).await,
                Err(HistoryError::Unverified)
            ));
            harness.accept(&with_dm(base, false)).await.unwrap();
            let regranted = harness.session.history.verified_channel(DM).await.unwrap();
            assert!(!Arc::ptr_eq(&old, &regranted));
            assert_ne!(harness.set(DM).await, old_set);
        })
        .unwrap();
    harness.finish();
}
