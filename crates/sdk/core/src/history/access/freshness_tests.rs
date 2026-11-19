//! Evidence freshness and revoke ordering: partial evidence never clears a backend denial,
//! forgetting a deleted channel never lets stale evidence regrant it, and an abandoned revoke
//! can never land after a newer one and hide the fresh set a regrant allocated.

use super::{
    Admission, Refetch, Transition,
    tests::{Evidence, Harness, RESTRICTED, snapshot},
};
use crate::{HistoryClosedReason, HistoryError};
use std::sync::Arc;

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
