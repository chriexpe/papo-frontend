//! Fresh badge subscriptions reflect R1 access restrictions synchronously; worker recounts and
//! notification eligibility remain outside these real-SQLite admission regressions.

use super::tests::{Evidence, Harness, OPEN, RESTRICTED, snapshot};
use crate::{BadgesSnapshot, ChannelBadges, HistoryClosedReason, Session, UnreadCount};
use std::{collections::BTreeMap, sync::Arc};

fn seed(harness: &Harness) -> (Session, Arc<BadgesSnapshot>) {
    let session = Session {
        inner: Arc::clone(&harness.session),
        client: harness.client.handle(),
    };
    // The harness accepts cache admission without publishing resources, so its badge worker
    // cannot recount this image while the test exercises the synchronous access boundary.
    assert_eq!(session.subscribe_resources().borrow().version, 0);
    let badges = Arc::new(BadgesSnapshot {
        generation: harness.generation,
        version: 1,
        channels_complete: true,
        channels: BTreeMap::from([
            (
                RESTRICTED,
                Arc::new(ChannelBadges {
                    mention_count: UnreadCount::Exact(1),
                    has_unread: Some(true),
                    direct_message: false,
                }),
            ),
            (
                OPEN,
                Arc::new(ChannelBadges {
                    mention_count: UnreadCount::Exact(2),
                    has_unread: Some(true),
                    direct_message: false,
                }),
            ),
        ]),
    });
    harness
        .session
        .badges
        .image
        .send_replace(Arc::clone(&badges));
    (session, badges)
}

#[test]
fn unknown_admission_immediately_redacts_fresh_subscriptions_without_republishing_noop() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    harness
        .client
        .block_on(async {
            harness
                .accept(&snapshot(harness.generation, Evidence::Allowed))
                .await
                .unwrap();
            let (session, seeded) = seed(&harness);
            let before = session.subscribe_badges();
            assert!(Arc::ptr_eq(&before.borrow(), &seeded));

            let unknown = snapshot(harness.generation, Evidence::Unknown);
            harness.accept(&unknown).await.unwrap();
            let redacted = session.subscribe_badges().borrow().clone();
            assert_eq!(
                redacted.channels[&RESTRICTED].as_ref(),
                &ChannelBadges {
                    mention_count: UnreadCount::AtLeast(0),
                    has_unread: None,
                    direct_message: false,
                }
            );
            assert_eq!(redacted.version, seeded.version + 1);
            assert!(redacted.channels_complete);
            assert!(before.has_changed().unwrap());
            assert!(Arc::ptr_eq(
                &redacted.channels[&OPEN],
                &seeded.channels[&OPEN]
            ));

            harness.accept(&unknown).await.unwrap();
            let repeated = session.subscribe_badges().borrow().clone();
            assert_eq!(repeated.version, redacted.version);
            assert!(Arc::ptr_eq(&repeated, &redacted));
        })
        .unwrap();
    harness.finish();
}

#[test]
fn denied_admission_immediately_removes_cached_badges_from_fresh_subscriptions() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    harness
        .client
        .block_on(async {
            harness
                .accept(&snapshot(harness.generation, Evidence::Allowed))
                .await
                .unwrap();
            let (session, seeded) = seed(&harness);
            let before = session.subscribe_badges();
            assert!(Arc::ptr_eq(&before.borrow(), &seeded));

            let denied = snapshot(harness.generation, Evidence::Denied);
            harness.accept(&denied).await.unwrap();
            let redacted = session.subscribe_badges().borrow().clone();
            assert!(!redacted.channels.contains_key(&RESTRICTED));
            assert_eq!(redacted.version, seeded.version + 1);
            assert!(redacted.channels_complete);
            assert!(before.has_changed().unwrap());
            assert!(Arc::ptr_eq(
                &redacted.channels[&OPEN],
                &seeded.channels[&OPEN]
            ));

            harness.accept(&denied).await.unwrap();
            let repeated = session.subscribe_badges().borrow().clone();
            assert_eq!(repeated.version, redacted.version);
            assert!(Arc::ptr_eq(&repeated, &redacted));
        })
        .unwrap();
    harness.finish();
}

#[test]
fn direct_denial_immediately_removes_cached_badges_and_repeated_denial_is_noop() {
    let directory = tempfile::tempdir().unwrap();
    let harness = Harness::open(&directory);
    harness
        .client
        .block_on(async {
            harness
                .accept(&snapshot(harness.generation, Evidence::Allowed))
                .await
                .unwrap();
            let (session, seeded) = seed(&harness);
            let before = session.subscribe_badges();
            assert!(Arc::ptr_eq(&before.borrow(), &seeded));

            super::deny(
                &harness.session,
                &harness.connection.client,
                &harness.connection.cache,
                RESTRICTED,
                None,
                HistoryClosedReason::PermissionDenied,
                harness.generation,
            )
            .await
            .unwrap();
            let redacted = session.subscribe_badges().borrow().clone();
            assert!(!redacted.channels.contains_key(&RESTRICTED));
            assert_eq!(redacted.version, seeded.version + 1);
            assert!(before.has_changed().unwrap());
            assert!(Arc::ptr_eq(
                &redacted.channels[&OPEN],
                &seeded.channels[&OPEN]
            ));

            super::deny(
                &harness.session,
                &harness.connection.client,
                &harness.connection.cache,
                RESTRICTED,
                None,
                HistoryClosedReason::PermissionDenied,
                harness.generation,
            )
            .await
            .unwrap();
            let repeated = session.subscribe_badges().borrow().clone();
            assert_eq!(repeated.version, redacted.version);
            assert!(Arc::ptr_eq(&repeated, &redacted));
        })
        .unwrap();
    harness.finish();
}
