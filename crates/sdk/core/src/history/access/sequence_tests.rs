//! Interleavings, failures and seeded sequences over the admission transitions: cancelled or
//! failed revokes keep the seal, a grant never admits over a concurrent unconfirmed denial or
//! a stray incarnation, and random decision sequences never revive a denied incarnation or set.

use super::{
    Admission, Transition,
    tests::{Evidence, Harness, OPEN, RESTRICTED, poll_once, snapshot},
};
use crate::{HistoryClosedReason, HistoryError, history::channel::Channel};
use papo_api::Uuid;
use proptest::{
    prelude::*,
    test_runner::{Config, RngAlgorithm, TestRng, TestRunner},
};
use std::sync::Arc;

/// The allowed snapshot with `OPEN` no longer listed.
fn without_open(generation: crate::Generation) -> crate::ResourceSnapshot {
    let mut snapshot = snapshot(generation, Evidence::Allowed);
    snapshot.channels.remove(OPEN);
    snapshot
}

#[test]
fn cancelled_or_failed_revoke_stays_sealed_and_regrant_writes_the_marker_first() {
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
            let old_set = harness.set(RESTRICTED).await;

            // Cancel the transition while its revoke is queued behind a stalled writer.
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
                super::evidence(&harness.session, super::Refetch::Complete).await,
            ));
            assert!(poll_once(&mut seal).is_pending());
            drop(seal);
            release.send(()).unwrap();
            stall.await.unwrap().unwrap();
            assert_eq!(
                harness.admission(RESTRICTED).await,
                Some(Admission::Denied {
                    reason: HistoryClosedReason::PermissionDenied,
                    durable: false
                })
            );

            // A failed revoke (the cache already serves a newer generation) keeps the barrier.
            let next = harness.session.epochs.advance().unwrap().generation;
            harness
                .connection
                .cache
                .history_transaction(next, |_| Ok(()))
                .await
                .unwrap();
            let regrant = snapshot(generation, Evidence::Allowed);
            assert!(
                Transition::begin(
                    &harness.session,
                    &harness.connection,
                    &regrant,
                    super::evidence(&harness.session, super::Refetch::Complete).await
                )
                .await
                .is_err()
            );
            assert!(matches!(
                harness.session.history.verified_channel(RESTRICTED).await,
                Err(HistoryError::Closed(HistoryClosedReason::PermissionDenied))
            ));

            // The next generation's regrant writes the marker first and gets a fresh set.
            let regrant = snapshot(next, Evidence::Allowed);
            let access = Transition::begin(
                &harness.session,
                &harness.connection,
                &regrant,
                super::evidence(&harness.session, super::Refetch::Complete).await,
            )
            .await
            .unwrap();
            harness.connection.cache.replace(&regrant).await.unwrap();
            access
                .finish(&harness.session, &harness.connection)
                .await
                .unwrap();
            let regranted = harness
                .connection
                .cache
                .history_authorize(*RESTRICTED.as_bytes(), next, false)
                .await
                .unwrap();
            assert_ne!(regranted, old_set);
        })
        .unwrap();
    harness.finish();
}

#[test]
fn same_session_regrant_after_an_unpersisted_denial_never_reuses_the_old_set() {
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
            let old_set = harness.set(RESTRICTED).await;
            let old = harness
                .session
                .history
                .verified_channel(RESTRICTED)
                .await
                .unwrap();
            // Seal without letting the durable revoke run.
            {
                let mut channels = harness.session.history.channels.lock().await;
                let decisions = crate::permissions::read_access::ReadDecisions::evaluate(
                    &snapshot(generation, Evidence::Denied),
                    Uuid::nil(),
                );
                let mut admissions = harness.session.history.admission.write().await;
                admissions.seal(&decisions, &channels);
                channels.retain(|_, channel| !channel.is_closed());
            }
            harness
                .accept(&snapshot(generation, Evidence::Allowed))
                .await
                .unwrap();
            let regranted = harness
                .session
                .history
                .verified_channel(RESTRICTED)
                .await
                .unwrap();
            assert!(!Arc::ptr_eq(&old, &regranted));
            assert!(old.check().is_err());
            assert_ne!(harness.set(RESTRICTED).await, old_set);
        })
        .unwrap();
    harness.finish();
}

#[test]
fn grant_never_admits_over_a_denial_that_lands_while_it_is_revoking() {
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
            // OPEN leaves the list without a confirmed marker, so the next grant revokes it.
            {
                let mut admissions = harness.session.history.admission.write().await;
                admissions.deny(OPEN, HistoryClosedReason::ChannelRemoved);
            }
            let regrant = snapshot(generation, Evidence::Allowed);
            let access = Transition::begin(
                &harness.session,
                &harness.connection,
                &regrant,
                super::evidence(&harness.session, super::Refetch::Complete).await,
            )
            .await
            .unwrap();
            harness.connection.cache.replace(&regrant).await.unwrap();

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
            let mut finish = Box::pin(access.finish(&harness.session, &harness.connection));
            assert!(poll_once(&mut finish).is_pending());
            // A backend 403 for RESTRICTED arrives while the grant waits on OPEN's revoke.
            harness
                .session
                .history
                .admission
                .write()
                .await
                .deny(RESTRICTED, HistoryClosedReason::Forbidden);
            release.send(()).unwrap();
            stall.await.unwrap().unwrap();
            finish.await.unwrap();
            assert_eq!(
                harness.admission(RESTRICTED).await,
                Some(Admission::Denied {
                    reason: HistoryClosedReason::Forbidden,
                    durable: false
                })
            );
            assert_eq!(harness.admission(OPEN).await, Some(Admission::Allowed));
        })
        .unwrap();
    harness.finish();
}

#[test]
fn grant_closes_a_stray_incarnation_instead_of_adopting_its_cached_state() {
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
            harness.accept(&without_open(generation)).await.unwrap();
            // Something outside admission made an Arc for the removed channel.
            let stray = harness.session.history.channel(OPEN).await;
            harness
                .accept(&snapshot(generation, Evidence::Allowed))
                .await
                .unwrap();
            let granted = harness
                .session
                .history
                .verified_channel(OPEN)
                .await
                .unwrap();
            assert!(!Arc::ptr_eq(&stray, &granted));
            assert!(stray.is_closed());
        })
        .unwrap();
    harness.finish();
}

/// Any accepted sequence of decisions, with stale incarnations held across it, never revives
/// an incarnation closed by a denial and never hands a denied channel its pre-denial set.
#[test]
fn random_accepted_transitions_never_resurrect_a_denied_incarnation_or_set() {
    let mut runner = TestRunner::new_with_rng(
        Config {
            cases: 24,
            failure_persistence: None,
            ..Config::default()
        },
        TestRng::deterministic_rng(RngAlgorithm::ChaCha),
    );
    let step = prop_oneof![
        Just(Evidence::Allowed),
        Just(Evidence::Denied),
        Just(Evidence::Unknown)
    ];
    runner
        .run(&proptest::collection::vec(step, 1..10), |steps| {
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
                    let mut set = Some(harness.set(RESTRICTED).await);
                    let mut held: Vec<Arc<Channel>> = vec![
                        harness
                            .session
                            .history
                            .verified_channel(RESTRICTED)
                            .await
                            .unwrap(),
                    ];
                    let mut retired_sets = Vec::new();
                    for evidence in steps {
                        harness
                            .accept(&snapshot(generation, evidence))
                            .await
                            .unwrap();
                        let opened = harness.session.history.verified_channel(RESTRICTED).await;
                        match evidence {
                            Evidence::Allowed => {
                                let channel = opened.unwrap();
                                let current = harness.set(RESTRICTED).await;
                                assert!(!retired_sets.contains(&current));
                                set = Some(current);
                                if !held.iter().any(|known| Arc::ptr_eq(known, &channel)) {
                                    held.push(channel);
                                }
                            }
                            Evidence::Denied => {
                                assert!(opened.is_err());
                                retired_sets.extend(set.take());
                            }
                            Evidence::Unknown => assert!(opened.is_err()),
                        }
                        let live: Vec<_> = held
                            .iter()
                            .filter(|channel| channel.check().is_ok())
                            .collect();
                        // At most one incarnation is ever live, and only while Allowed.
                        assert!(live.len() <= 1);
                        if !matches!(evidence, Evidence::Allowed) {
                            assert!(live.is_empty());
                        }
                        for channel in &held {
                            if channel.is_closed() {
                                assert!(channel.check().is_err());
                            }
                        }
                    }
                })
                .unwrap();
            harness.finish();
            Ok(())
        })
        .unwrap();
}

#[test]
fn completed_backend_denial_during_a_transition_is_not_reopened_by_its_older_evidence() {
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
            let old_set = harness.set(RESTRICTED).await;
            let old = harness
                .session
                .history
                .verified_channel(RESTRICTED)
                .await
                .unwrap();

            // A producer starts fetching, then evaluates and writes evidence that still allows.
            let fetched = super::evidence(&harness.session, super::Refetch::Complete).await;
            let allowed = snapshot(generation, Evidence::Allowed);
            let transition =
                Transition::begin(&harness.session, &harness.connection, &allowed, fetched)
                    .await
                    .unwrap();
            harness.connection.cache.replace(&allowed).await.unwrap();

            // Meanwhile a history request is refused with 403 and its denial fully completes.
            old.close(HistoryClosedReason::Forbidden);
            super::deny(
                &harness.session,
                &harness.client.handle(),
                &harness.connection.cache,
                RESTRICTED,
                HistoryClosedReason::Forbidden,
                generation,
            )
            .await
            .unwrap();
            assert_eq!(
                harness.admission(RESTRICTED).await,
                Some(Admission::Denied {
                    reason: HistoryClosedReason::Forbidden,
                    durable: true
                })
            );

            // The older evidence finishes without reopening the channel.
            transition
                .finish(&harness.session, &harness.connection)
                .await
                .unwrap();
            assert!(matches!(
                harness.session.history.verified_channel(RESTRICTED).await,
                Err(HistoryError::Closed(HistoryClosedReason::Forbidden))
            ));

            // Evidence fetched after the denial may regrant, on a fresh cache set.
            harness
                .accept(&snapshot(generation, Evidence::Allowed))
                .await
                .unwrap();
            let regranted = harness
                .session
                .history
                .verified_channel(RESTRICTED)
                .await
                .unwrap();
            assert!(!Arc::ptr_eq(&old, &regranted));
            assert_ne!(harness.set(RESTRICTED).await, old_set);
        })
        .unwrap();
    harness.finish();
}

/// The durable boundary is the marker commit. A transition cancelled mid-revoke, with its
/// connection generation replaced, still commits the marker, so a fresh session on the
/// reopened SQLite cache cannot reuse the old set.
#[test]
fn cancelled_revoke_with_a_replaced_generation_still_commits_and_survives_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    let generation = harness.generation;
    let old_set = harness
        .client
        .block_on(async {
            harness
                .accept(&snapshot(generation, Evidence::Allowed))
                .await
                .unwrap();
            let old_set = harness.set(RESTRICTED).await;
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
                super::evidence(&harness.session, super::Refetch::Complete).await,
            ));
            assert!(poll_once(&mut seal).is_pending());
            drop(seal);
            let next = harness.session.epochs.advance().unwrap().generation;
            release.send(()).unwrap();
            stall.await.unwrap().unwrap();
            // The writer is FIFO behind the fence: this returns after the detached marker.
            harness
                .connection
                .cache
                .history_transaction(next, |_| Ok(()))
                .await
                .unwrap();
            // In memory the denial is still unconfirmed, so this session would retry it.
            assert_eq!(
                harness.admission(RESTRICTED).await,
                Some(Admission::Denied {
                    reason: HistoryClosedReason::PermissionDenied,
                    durable: false
                })
            );
            old_set
        })
        .unwrap();
    harness.finish();

    let reopened = Harness::open(&directory);
    let generation = reopened.generation;
    reopened
        .client
        .block_on(async {
            reopened
                .accept(&snapshot(generation, Evidence::Allowed))
                .await
                .unwrap();
            assert_ne!(reopened.set(RESTRICTED).await, old_set);
        })
        .unwrap();
    reopened.finish();
}
