//! A live resource writer owns installation even after its caller drops or its epoch changes.

use super::*;
use crate::{Session, Uuid, notifications::tests::fixture, resource_admission::Admission};
use papo_api::wire::{Channel, ChannelList, Server, UserSummary};

async fn connection(session: &Session) -> ResourceConnection {
    ResourceConnection {
        client: session.client.clone(),
        cache: session
            .client
            .inner
            .caches
            .open(session.account())
            .await
            .unwrap(),
        rest: papo_api::RestClient::new(session.inner.address.as_str()).unwrap(),
    }
}

fn complete(session: &Session) -> AdmittedMutation {
    let admission = Admission::new(&session.inner);
    let own = session.account().user();
    let server = admission
        .server(Server {
            id: Some(Uuid::from_u128(10).to_string()),
            owner_id: Some(own.to_string()),
            ..Default::default()
        })
        .unwrap();
    let mut mutation = channels(session, "committed");
    mutation.complete = true;
    mutation.server = Some(server);
    mutation.roles = Some(Vec::new());
    mutation.roster = Some(
        admission
            .summaries(
                &[own],
                vec![UserSummary {
                    id: Some(own.to_string()),
                    roles: Some(Vec::new()),
                    ..Default::default()
                }],
            )
            .unwrap()
            .present,
    );
    mutation.dms = Some(Vec::new());
    mutation.blocks = Some(Vec::new());
    mutation
}

fn channels(session: &Session, name: &str) -> AdmittedMutation {
    let wire = Channel {
        id: Some(Uuid::from_u128(2).to_string()),
        r#type: Some("text".into()),
        name: Some(name.into()),
        permissions: Some(Vec::new()),
        ..Default::default()
    };
    AdmittedMutation {
        channels: Some(
            Admission::new(&session.inner)
                .channels(ChannelList {
                    channels: Some(vec![wire]),
                })
                .unwrap(),
        ),
        ..Default::default()
    }
}

async fn submit(
    session: &Session,
    connection: &ResourceConnection,
    facts: AdmittedMutation,
) -> Result<Outcome, ClientError> {
    let guard = reserve(&session.inner).await;
    let refetch = if facts.complete {
        Refetch::Complete
    } else {
        Refetch::Partial
    };
    let stamp = FetchStamp::capture(&session.inner, session.generation(), refetch).await?;
    commit(&session.inner, connection, guard, stamp, Ok(facts)).await
}

#[test]
fn caller_cancellation_after_revision_switch_retains_writer_until_publication() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let connection = connection(&session).await;
            assert_eq!(
                submit(&session, &connection, complete(&session))
                    .await
                    .unwrap(),
                Outcome::Accepted
            );
            let reading = session.inner.reading.serial.lock().await;
            let before = session.inner.resources.borrow().clone();
            let channel = Uuid::from_u128(2);
            let task = tokio::spawn({
                let session = session.clone();
                let connection = connection.clone();
                async move { submit(&session, &connection, channels(&session, "next")).await }
            });
            // The cache fence orders this read after the submitted SQL batch. The reading
            // lock prevents the post-switch install from publishing before cancellation.
            loop {
                let visible = connection.cache.load(session.generation()).await.unwrap();
                if visible
                    .channels
                    .get(channel)
                    .and_then(|row| row.name.as_deref())
                    == Some("next")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert!(Arc::ptr_eq(&before, &session.inner.resources.borrow()));
            assert!(session.inner.resource_serial.try_lock().is_err());
            drop(reading);
            let _guard = session.inner.resource_serial.lock().await;
            assert!(session.inner.resource_recovery.borrow().is_none());
            assert_eq!(
                session
                    .inner
                    .resources
                    .borrow()
                    .channels
                    .get(channel)
                    .unwrap()
                    .name
                    .as_deref(),
                Some("next")
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn failure_after_revision_switch_blocks_partial_writes_and_preserves_the_disk_image() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let connection = connection(&session).await;
            // Reading must recover the same account session. Replacing the pool entry forces
            // downstream reconciliation to fail after the resource revision has switched.
            let stale = crate::session::SessionInner::new(
                session.account(),
                session.inner.address.clone(),
                session.inner.key,
                Arc::clone(&session.client.inner.pool.next),
                session.client.inner.pool.root.child_token(),
                crate::AuthState::Ready,
            );
            let orphan = Session {
                client: session.client.clone(),
                inner: stale,
            };
            let generation = orphan.generation();
            let result = submit(&orphan, &connection, complete(&orphan)).await;
            assert!(matches!(result, Err(ClientError::Superseded)));

            assert_eq!(*orphan.inner.resource_recovery.borrow(), Some(generation));
            assert_eq!(orphan.generation(), generation);
            assert!(orphan.inner.resources.borrow().channels.is_empty());
            let channel = Uuid::from_u128(2);
            assert_eq!(
                orphan.inner.history.admission.read().await.channel(channel),
                Some(access::ChannelAdmission::Quarantined)
            );
            assert_eq!(
                submit(&orphan, &connection, channels(&orphan, "partial"))
                    .await
                    .unwrap(),
                Outcome::ConcurrentChange
            );
            let disk = connection.cache.load(orphan.generation()).await.unwrap();
            assert_eq!(
                disk.channels.get(channel).unwrap().name.as_deref(),
                Some("committed")
            );
            orphan.inner.stop().await;
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn an_already_denied_channel_cannot_regrant_from_an_overwrite_observed_to_change_or_disappear() {
    use crate::permissions::read_access::{ReadAccess, ReadDecisions};
    use papo_api::wire::{
        ChannelPermissionEntry, ChannelPermissions, Role, RolePermissions, RoleSummary,
    };
    let own = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let a = Uuid::from_u128(3);
    let b = Uuid::from_u128(4);
    let row = |role: Uuid| Channel {
        id: Some(channel.to_string()),
        r#type: Some("text".into()),
        permissions: Some(vec![ChannelPermissionEntry {
            role_id: Some(role.to_string()),
            permissions: Some(ChannelPermissions {
                read_channel: Some(true),
                ..Default::default()
            }),
            ..Default::default()
        }]),
        ..Default::default()
    };
    let member = |roles| UserSummary {
        id: Some(own.to_string()),
        roles: Some(roles),
        ..Default::default()
    };
    let base = ResourceSnapshot::fixture(
        Generation(1),
        Some(Server {
            id: Some(Uuid::from_u128(10).to_string()),
            ..Default::default()
        }),
        vec![row(a)],
        [a, b]
            .map(|id| Role {
                id: Some(id.to_string()),
                permissions: Some(RolePermissions::default()),
                ..Default::default()
            })
            .into(),
        vec![member(vec![])],
        vec![],
        vec![],
        vec![],
        own,
    )
    .unwrap();
    assert_eq!(
        ReadDecisions::evaluate(&base, own).get(channel),
        Some(ReadAccess::Denied)
    );
    let diagnostics = crate::resource_admission::Diagnostics::default();
    let admit = Admission::for_test(own, &diagnostics);
    for changed in [vec![], vec![row(b)]] {
        let observed = AdmittedMutation {
            channels: Some(
                admit
                    .channels(ChannelList {
                        channels: Some(changed),
                    })
                    .unwrap(),
            ),
            ..Default::default()
        };
        let mut gap = base.clone();
        gap.gaps
            .observe(CacheResource::Channels, &base, &observed, own);
        let assignment = admit
            .summaries(
                &[own],
                vec![member(vec![RoleSummary {
                    id: Some(a.to_string()),
                    ..Default::default()
                }])],
            )
            .unwrap();
        let next = AdmittedMutation {
            summaries: vec![assignment],
            ..Default::default()
        }
        .apply(&gap, own)
        .unwrap();
        assert_eq!(
            ReadDecisions::evaluate(&next, own).get(channel),
            Some(ReadAccess::Unknown)
        );
    }
}

#[test]
fn cancellation_waiting_for_admission_cannot_drop_an_interim_resource_seal() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let connection = connection(&session).await;
            submit(&session, &connection, complete(&session))
                .await
                .unwrap();
            let guard = reserve(&session.inner).await;
            let mut stamp =
                FetchStamp::capture(&session.inner, session.generation(), Refetch::Partial)
                    .await
                    .unwrap();
            let counter = stamp.observation;
            let admission = session.inner.history.admission.write().await;
            let caller = tokio::spawn({
                let session = session.clone();
                async move {
                    let missing = AdmittedMutation {
                        channels: Some(Vec::new()),
                        ..Default::default()
                    };
                    stamp
                        .observing(&session.inner, Some(&guard))
                        .before_rest(CacheResource::Channels, &missing)
                        .await
                        .map_err(|failure| failure.error)
                }
            });
            while session.inner.resource_observation.load(Ordering::Acquire) == counter {
                assert!(!caller.is_finished());
                tokio::task::yield_now().await;
            }
            caller.abort();
            assert!(caller.await.unwrap_err().is_cancelled());
            assert!(session.inner.resource_serial.try_lock().is_err());
            drop(admission);
            let _guard = session.inner.resource_serial.lock().await;
            assert_eq!(
                session
                    .inner
                    .history
                    .admission
                    .read()
                    .await
                    .channel(Uuid::from_u128(2)),
                Some(access::ChannelAdmission::Quarantined)
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}
