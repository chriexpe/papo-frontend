//! Cold display bootstrap retains cached groups and independently observed evidence gaps.

use super::*;

#[test]
fn rejection_before_first_cache_load_preserves_both_cached_facts_and_the_evidence_gap() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let generation = session.generation();
            // The notification harness starts with a channel in memory; begin with
            // the same unpublished resource cell a restored session receives.
            session
                .inner
                .resources
                .send_replace(Arc::new(ResourceSnapshot::empty(generation)));
            let connection = connection(&session).await;
            let cached = complete(&session)
                .apply(
                    &ResourceSnapshot::empty(generation),
                    session.account().user(),
                )
                .unwrap();
            connection.cache.replace(&cached).await.unwrap();
            let guard = reserve(&session.inner).await;
            let stamp = FetchStamp::capture(&session.inner, generation, Refetch::Partial)
                .await
                .unwrap();
            let failure =
                ScopedFailure::for_group(CacheResource::Channels, papo_api::Error::Decode.into());
            let result = commit(&session.inner, &connection, guard, stamp, Err(failure)).await;
            assert!(matches!(
                result,
                Err(ClientError::Api(papo_api::Error::Decode))
            ));
            let rejected = session.inner.resources.borrow().clone();
            assert!(rejected.version > 0);
            assert!(rejected.channels.is_empty());
            assert!(!rejected.has_display_baseline);

            let guard = reserve(&session.inner).await;
            connection
                .bootstrap_display(&session.inner, generation, &guard)
                .await
                .unwrap();
            let image = session.inner.resources.borrow().clone();
            assert!(image.channels.get(Uuid::from_u128(2)).is_some());
            assert!(image.gaps.rejected(CacheResource::Channels));
            assert!(image.has_display_baseline);
            assert!(!access::accepted(&session.inner, generation).await);
        })
        .unwrap();
    client.shutdown().unwrap();
}
