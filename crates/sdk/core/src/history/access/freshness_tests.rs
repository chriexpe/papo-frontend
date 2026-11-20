//! Evidence freshness and revoke ordering: partial evidence never clears a backend denial,
//! forgetting a deleted channel never lets stale evidence regrant it, and an abandoned revoke
//! can never land after a newer one and hide the fresh set a regrant allocated.

use super::{
    Admission, Refetch, Transition,
    tests::{Evidence, Harness, OPEN, RESTRICTED, poll_once, snapshot},
};
use crate::{HistoryClosedReason, HistoryError};
use std::sync::Arc;

#[test]
fn backend_denial_filters_stale_reading_published_in_the_next_generation() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    let generation = harness.generation;
    harness
        .client
        .block_on(async {
            let session = &harness.session;
            let reading = &session.reading;
            harness
                .accept(&snapshot(generation, Evidence::Allowed))
                .await
                .unwrap();
            reading
                .publish(
                    session,
                    crate::ReadingSnapshot {
                        generation,
                        version: 0,
                        channels: [
                            (RESTRICTED, crate::ReadState::default()),
                            (OPEN, crate::ReadState::default()),
                        ]
                        .into(),
                    },
                )
                .await
                .unwrap();
            let mut stale = (**reading.image.borrow()).clone();
            assert!(stale.channels.contains_key(&RESTRICTED));
            assert!(stale.channels.contains_key(&OPEN));

            // Hold the cache writer so the denial seals admission but cannot publish
            // reading cleanup while the stale image moves to the next generation.
            let cache = Arc::clone(&harness.connection.cache);
            let (entered, waiting) = tokio::sync::oneshot::channel();
            let (release, blocked) = std::sync::mpsc::channel::<()>();
            let blocked_writer = tokio::spawn(async move {
                cache
                    .history_transaction(generation, move |_| {
                        entered.send(()).unwrap();
                        let _ = blocked.recv();
                        Ok(())
                    })
                    .await
            });
            waiting.await.unwrap();
            let mut denial = Box::pin(super::deny(
                session,
                &harness.connection.client,
                &harness.connection.cache,
                RESTRICTED,
                None,
                HistoryClosedReason::Forbidden,
                generation,
            ));
            assert!(poll_once(&mut denial).is_pending());
            assert_eq!(
                harness.admission(RESTRICTED).await,
                Some(Admission::Denied {
                    reason: HistoryClosedReason::Forbidden,
                    durable: false,
                })
            );
            assert!(reading.image.borrow().channels.contains_key(&RESTRICTED));

            stale.generation = session.epochs.advance().unwrap().generation;
            let next_generation = stale.generation;
            reading.publish(session, stale).await.unwrap();
            {
                let published = reading.image.borrow();
                assert_eq!(published.generation, next_generation);
                assert!(!published.channels.contains_key(&RESTRICTED));
                assert!(published.channels.contains_key(&OPEN));
            }
            release.send(()).unwrap();
            blocked_writer.await.unwrap().unwrap();
            assert!(matches!(denial.await, Err(crate::ClientError::Superseded)));
        })
        .unwrap();
    harness.finish();
}

#[test]
fn backend_denial_is_cleared_only_by_a_complete_refetch_that_started_after_it() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    let generation = harness.generation;
    harness
        .client
        .block_on(async {
            let allowed = snapshot(generation, Evidence::Allowed);
            harness.accept(&allowed).await.unwrap();
            let old = harness
                .session
                .history
                .verified_channel(RESTRICTED)
                .await
                .unwrap();
            old.close(HistoryClosedReason::Forbidden);
            super::deny(
                &harness.session,
                &harness.client.handle(),
                &harness.connection.cache,
                RESTRICTED,
                None,
                HistoryClosedReason::Forbidden,
                generation,
            )
            .await
            .unwrap();

            // A gateway or operation refresh after the 403 patches part of an image older
            // than it; its still-allowing decision cannot explain away the backend refusal.
            let partial = super::evidence(&harness.session, Refetch::Partial).await;
            let transition =
                Transition::begin(&harness.session, &harness.connection, &allowed, partial)
                    .await
                    .unwrap();
            harness.connection.cache.replace(&allowed).await.unwrap();
            transition
                .finish(&harness.session, &harness.connection)
                .await
                .unwrap();
            assert!(matches!(
                harness.session.history.verified_channel(RESTRICTED).await,
                Err(HistoryError::Closed(HistoryClosedReason::Forbidden))
            ));

            // A complete refetch that started after the 403 may regrant.
            harness.accept(&allowed).await.unwrap();
            assert!(
                harness
                    .session
                    .history
                    .verified_channel(RESTRICTED)
                    .await
                    .is_ok()
            );
        })
        .unwrap();
    harness.finish();
}

#[test]
fn a_forgotten_deleted_channel_is_not_regranted_by_evidence_from_before_its_deletion() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    let generation = harness.generation;
    harness
        .client
        .block_on(async {
            let allowed = snapshot(generation, Evidence::Allowed);
            harness.accept(&allowed).await.unwrap();
            // A refresh starts fetching; its list still contains the channel.
            let stale = super::evidence(&harness.session, Refetch::Complete).await;
            // The gateway deletes the channel; the revoke completes and the entry is forgotten.
            super::deny(
                &harness.session,
                &harness.client.handle(),
                &harness.connection.cache,
                RESTRICTED,
                None,
                HistoryClosedReason::ChannelDeleted,
                generation,
            )
            .await
            .unwrap();
            assert_eq!(harness.admission(RESTRICTED).await, None);
            let transition =
                Transition::begin(&harness.session, &harness.connection, &allowed, stale)
                    .await
                    .unwrap();
            harness.connection.cache.replace(&allowed).await.unwrap();
            transition
                .finish(&harness.session, &harness.connection)
                .await
                .unwrap();
            assert_eq!(harness.admission(RESTRICTED).await, None);
        })
        .unwrap();
    harness.finish();
}

#[test]
fn an_abandoned_revoke_commits_before_a_newer_one_and_never_hides_the_regranted_set() {
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
            harness.set(RESTRICTED).await;
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
            // The first revoke is abandoned by its caller; the second is awaited.
            let cache = &harness.connection.cache;
            drop(cache.history_revoke_detached(vec![*RESTRICTED.as_bytes()]));
            let newer = cache.history_revoke_detached(vec![*RESTRICTED.as_bytes()]);
            release.send(()).unwrap();
            stall.await.unwrap().unwrap();
            newer.await.unwrap().unwrap();
            let regranted = harness.set(RESTRICTED).await;
            // Flush anything still queued behind the fence, then the regrant must stand.
            cache
                .history_transaction(generation, |_| Ok(()))
                .await
                .unwrap();
            assert_eq!(harness.set(RESTRICTED).await, regranted);
            assert_eq!(
                harness.admission(RESTRICTED).await,
                Some(Admission::Allowed)
            );
        })
        .unwrap();
    harness.finish();
}

#[test]
fn grant_waiting_for_the_channel_map_cannot_make_a_superseded_connection_warm() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    harness
        .client
        .block_on(async {
            let allowed = snapshot(harness.generation, Evidence::Allowed);
            let evidence = super::evidence(&harness.session, Refetch::Complete).await;
            let transition =
                Transition::begin(&harness.session, &harness.connection, &allowed, evidence)
                    .await
                    .unwrap();
            harness.connection.cache.replace(&allowed).await.unwrap();

            let channels = harness.session.history.channels.lock().await;
            let mut grant = Box::pin(transition.finish(&harness.session, &harness.connection));
            assert!(super::tests::poll_once(&mut grant).is_pending());
            harness.session.epochs.advance().unwrap();
            drop(channels);

            assert!(matches!(grant.await, Err(crate::ClientError::Superseded)));
            assert_eq!(harness.admission(RESTRICTED).await, None);
            assert!(harness.session.history.connection.read().await.is_none());
        })
        .unwrap();
    harness.finish();
}

#[test]
fn seal_waiting_for_the_channel_map_cannot_revoke_the_next_epochs_warm_history() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    harness
        .client
        .block_on(async {
            harness
                .accept(&snapshot(harness.generation, Evidence::Allowed))
                .await
                .unwrap();
            let set = harness.set(RESTRICTED).await;
            let channel = harness
                .session
                .history
                .verified_channel(RESTRICTED)
                .await
                .unwrap();
            let denied = snapshot(harness.generation, Evidence::Denied);
            let evidence = super::evidence(&harness.session, Refetch::Complete).await;
            let channels = harness.session.history.channels.lock().await;
            let mut seal = Box::pin(Transition::begin(
                &harness.session,
                &harness.connection,
                &denied,
                evidence,
            ));
            assert!(super::tests::poll_once(&mut seal).is_pending());
            harness.session.epochs.advance().unwrap();
            drop(channels);

            assert!(matches!(seal.await, Err(crate::ClientError::Superseded)));
            assert_eq!(
                harness.admission(RESTRICTED).await,
                Some(Admission::Allowed)
            );
            assert!(channel.check().is_ok());
            assert_eq!(harness.set(RESTRICTED).await, set);
        })
        .unwrap();
    harness.finish();
}

#[test]
fn delayed_history_refusal_cannot_revoke_a_replacement_channel_or_epoch() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    harness
        .client
        .block_on(async {
            let allowed = snapshot(harness.generation, Evidence::Allowed);
            harness.accept(&allowed).await.unwrap();
            let previous = harness
                .session
                .history
                .verified_channel(RESTRICTED)
                .await
                .unwrap();
            harness
                .accept(&snapshot(harness.generation, Evidence::Denied))
                .await
                .unwrap();
            harness.accept(&allowed).await.unwrap();
            let current = harness
                .session
                .history
                .verified_channel(RESTRICTED)
                .await
                .unwrap();
            let set = harness.set(RESTRICTED).await;

            let refusal = |incarnation| {
                super::deny(
                    &harness.session,
                    &harness.connection.client,
                    &harness.connection.cache,
                    RESTRICTED,
                    Some(incarnation),
                    HistoryClosedReason::Forbidden,
                    harness.generation,
                )
            };
            assert!(matches!(
                refusal(&previous).await,
                Err(crate::ClientError::Superseded)
            ));
            let channels = harness.session.history.channels.lock().await;
            let mut pending = Box::pin(refusal(&current));
            assert!(super::tests::poll_once(&mut pending).is_pending());
            harness.session.epochs.advance().unwrap();
            drop(channels);
            assert!(matches!(pending.await, Err(crate::ClientError::Superseded)));
            assert_eq!(
                harness.admission(RESTRICTED).await,
                Some(Admission::Allowed)
            );
            assert!(current.check().is_ok());
            assert_eq!(harness.set(RESTRICTED).await, set);
        })
        .unwrap();
    harness.finish();
}
