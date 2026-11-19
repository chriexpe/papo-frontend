//! Reading-owned durable evidence, deferral-before-claim and provenance unit tests.

use super::{
    NotificationSettings,
    deliver::{self, DeliveryOutcome},
    policy::{self, PolicyResources},
    tests::{candidate, fixture},
};
use crate::{
    HistoryClosedReason, MessageKey, Uuid,
    read_points::{ServerReadPoint, UploadedPoint},
    reading::evidence::ReadEvidence,
};
use std::sync::Arc;

fn ledger_count(directory: &std::path::Path) -> i64 {
    let connection = rusqlite::Connection::open(directory.join("state.sqlite")).unwrap();
    connection
        .query_row("SELECT count(*) FROM notification_ledger", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
}

#[derive(Clone, Copy)]
enum DurableVariant {
    Local,
    Uploaded,
    Known,
    UnresolvedExactId,
}

async fn seed_durable(
    session: &crate::Session,
    channel: Uuid,
    generation: crate::Generation,
    exact_id: Uuid,
    variant: DurableVariant,
) {
    let registry = &session.client.inner.registry;
    let boundary = MessageKey {
        created_at_us: 150,
        message_id: Uuid::from_u128(150),
    };
    match variant {
        DurableVariant::Local => {
            registry
                .advance_local_read(Arc::clone(&session.inner), channel, generation, boundary)
                .await
                .unwrap();
        }
        DurableVariant::Uploaded => {
            registry
                .save_uploaded_read(
                    Arc::clone(&session.inner),
                    channel,
                    generation,
                    UploadedPoint {
                        server: ServerReadPoint {
                            message_id: boundary.message_id,
                            recorded_at: None,
                        },
                        created_at_us: Some(150),
                        known_point: None,
                    },
                )
                .await
                .unwrap();
        }
        DurableVariant::Known => {
            registry
                .save_uploaded_read(
                    Arc::clone(&session.inner),
                    channel,
                    generation,
                    UploadedPoint {
                        server: ServerReadPoint {
                            message_id: Uuid::from_u128(999),
                            recorded_at: Some("2099-01-01T00:00:00Z".into()),
                        },
                        created_at_us: None,
                        known_point: Some(boundary),
                    },
                )
                .await
                .unwrap();
        }
        DurableVariant::UnresolvedExactId => {
            registry
                .save_uploaded_read(
                    Arc::clone(&session.inner),
                    channel,
                    generation,
                    UploadedPoint {
                        server: ServerReadPoint {
                            message_id: exact_id,
                            recorded_at: Some("2099-01-01T00:00:00Z".into()),
                        },
                        created_at_us: None,
                        known_point: None,
                    },
                )
                .await
                .unwrap();
        }
    }
}

#[test]
fn durable_local_uploaded_known_and_unresolved_points_suppress_with_empty_reading_image() {
    for variant in [
        DurableVariant::Local,
        DurableVariant::Uploaded,
        DurableVariant::Known,
        DurableVariant::UnresolvedExactId,
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (mut client, session) = fixture(directory.path());
        client
            .block_on(async {
                let generation = session.generation();
                let mut read_candidate = candidate();
                read_candidate.message_point = Some(MessageKey {
                    created_at_us: 100,
                    message_id: read_candidate.message,
                });
                let mut unread_candidate = candidate();
                unread_candidate.message = Uuid::from_u128(300);
                unread_candidate.message_point = Some(MessageKey {
                    created_at_us: 200,
                    message_id: unread_candidate.message,
                });

                seed_durable(
                    &session,
                    read_candidate.channel,
                    generation,
                    read_candidate.message,
                    variant,
                )
                .await;
                // Cold state: reading image is still completely empty.
                assert!(session.reading_state(read_candidate.channel).is_none());
                let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());

                // Read candidate is suppressed AND intentionally consumes its ledger identity.
                assert!(matches!(
                    deliver::deliver_at(&session, generation, read_candidate, &resources, 1)
                        .await
                        .unwrap(),
                    DeliveryOutcome::Handled(None)
                ));
                assert_eq!(ledger_count(directory.path()), 1);

                // Unread counterpart delivers and claims its own ledger identity.
                assert!(matches!(
                    deliver::deliver_at(&session, generation, unread_candidate, &resources, 1)
                        .await
                        .unwrap(),
                    DeliveryOutcome::Handled(Some(_))
                ));
                assert_eq!(ledger_count(directory.path()), 2);
            })
            .unwrap();
        client.shutdown().unwrap();
    }
}

#[test]
fn in_memory_unresolved_id_and_inbox_read_flag_suppress_and_claim() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let generation = session.generation();
            let mut item = candidate();
            session.inner.reading.unresolved.lock().await.insert(
                item.channel,
                ServerReadPoint {
                    message_id: item.message,
                    recorded_at: Some("2099-01-01T00:00:00Z".into()),
                },
            );
            let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
            assert!(matches!(
                deliver::deliver_at(&session, generation, item.clone(), &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(None)
            ));
            assert_eq!(ledger_count(directory.path()), 1);

            // Different message ID is NOT suppressed by the unresolved mark alone.
            item.message = Uuid::from_u128(90);
            assert!(matches!(
                deliver::deliver_at(&session, generation, item.clone(), &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(Some(_))
            ));
            assert_eq!(ledger_count(directory.path()), 2);

            // Independent backend inbox `read=true` still suppresses and claims.
            item.message = Uuid::from_u128(91);
            item.read = true;
            assert!(
                policy::decide(
                    &session,
                    generation,
                    &NotificationSettings::default(),
                    &papo_api::wire::Notifications::default(),
                    &item,
                    &resources,
                    ReadEvidence::Unread,
                )
                .is_none()
            );
            assert!(matches!(
                deliver::deliver_at(&session, generation, item, &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(None)
            ));
            assert_eq!(ledger_count(directory.path()), 3);
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn not_ready_defers_before_claim_then_delivers_or_suppresses_once_ready() {
    for already_read in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let (mut client, session) = fixture(directory.path());
        client
            .block_on(async {
                let generation = session.generation();
                let mut item = candidate();
                item.message_point = Some(MessageKey {
                    created_at_us: 100,
                    message_id: item.message,
                });
                if already_read {
                    session
                        .client
                        .inner
                        .registry
                        .advance_local_read(
                            Arc::clone(&session.inner),
                            item.channel,
                            generation,
                            MessageKey {
                                created_at_us: 200,
                                message_id: Uuid::from_u128(200),
                            },
                        )
                        .await
                        .unwrap();
                }

                // Quarantine makes read evidence NotReady: must defer before any ledger claim.
                session.inner.reading.positions.quarantine(&[item.channel]);
                let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
                assert!(matches!(
                    deliver::deliver_at(&session, generation, item.clone(), &resources, 0)
                        .await
                        .unwrap(),
                    DeliveryOutcome::Deferred
                ));
                assert_eq!(ledger_count(directory.path()), 0);

                // Once quarantine lifts, the candidate resolves cleanly.
                session.inner.reading.positions.release(&[item.channel]);
                let outcome = deliver::deliver_at(&session, generation, item, &resources, 0)
                    .await
                    .unwrap();
                if already_read {
                    assert!(matches!(outcome, DeliveryOutcome::Handled(None)));
                } else {
                    assert!(matches!(outcome, DeliveryOutcome::Handled(Some(_))));
                }
                assert_eq!(ledger_count(directory.path()), 1);
            })
            .unwrap();
        client.shutdown().unwrap();
    }
}

#[test]
fn missing_candidate_point_defers_when_ordered_read_point_exists_then_resolves() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let generation = session.generation();
            let mut item = candidate();
            item.message_point = None;
            session
                .client
                .inner
                .registry
                .advance_local_read(
                    Arc::clone(&session.inner),
                    item.channel,
                    generation,
                    MessageKey {
                        created_at_us: 100,
                        message_id: Uuid::from_u128(100),
                    },
                )
                .await
                .unwrap();
            let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
            assert!(matches!(
                deliver::deliver_at(&session, generation, item.clone(), &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Deferred
            ));
            assert_eq!(ledger_count(directory.path()), 0);

            // Once canonical key resolves above local_point, it delivers and claims.
            item.message_point = Some(MessageKey {
                created_at_us: 200,
                message_id: item.message,
            });
            assert!(matches!(
                deliver::deliver_at(&session, generation, item, &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(Some(_))
            ));
            assert_eq!(ledger_count(directory.path()), 1);
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn denied_channel_suppresses_with_background_channel_dm_resources() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let generation = session.generation();
            let item = candidate();
            let bg = PolicyResources::channels_and_dms(
                generation,
                (*session.inner.resources.borrow().channels).clone(),
                Vec::new(),
            );
            assert_eq!(bg.generation(), generation);
            assert!(!bg.channels().is_empty());

            session.inner.reading.positions.revoke(
                &[item.channel],
                generation,
                HistoryClosedReason::PermissionDenied,
            );
            assert!(matches!(
                deliver::deliver_at(&session, generation, item, &bg, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(None)
            ));
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn ineligible_quarantined_message_does_not_defer_and_reply_upgrades_queued_entry() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let generation = session.generation();
            let mut ordinary = candidate();
            ordinary.triggered = false;
            ordinary.message_point = Some(MessageKey {
                created_at_us: 100,
                message_id: ordinary.message,
            });
            session
                .inner
                .reading
                .positions
                .quarantine(&[ordinary.channel]);
            let resources = PolicyResources::from_snapshot(&session.inner.resources.borrow());
            assert!(matches!(
                deliver::deliver_at(&session, generation, ordinary.clone(), &resources, 0)
                    .await
                    .unwrap(),
                DeliveryOutcome::Handled(None)
            ));
            assert_eq!(ledger_count(directory.path()), 0);

            // If an ambiguous channel+DM entry was already queued non-triggered, a matching
            // triggered reply must upgrade it rather than being dropped by message-ID dedupe.
            let mut queue = std::collections::VecDeque::new();
            assert!(!super::live_queue::enqueue_deferred(
                &mut queue,
                generation,
                ordinary.clone(),
            ));
            let mut reply = candidate();
            reply.message = ordinary.message;
            reply.triggered = true;
            reply.message_point = None;
            assert!(!super::live_queue::enqueue_deferred(
                &mut queue, generation, reply,
            ));
            assert_eq!(queue.len(), 1);
            assert!(queue[0].1.triggered);
            assert_eq!(queue[0].1.message_point, ordinary.message_point);
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn validate_target_rejects_quarantined_and_retained_denied_channels() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    let generation = session.generation();
    let channel = candidate().channel;
    let target = super::NotificationTarget {
        account: session.account(),
        generation,
        channel,
        message: candidate().message,
    };
    assert!(session.notifications().validate_target(&target).is_ok());

    session.inner.reading.positions.quarantine(&[channel]);
    assert!(session.notifications().validate_target(&target).is_err());
    session.inner.reading.positions.release(&[channel]);
    assert!(session.notifications().validate_target(&target).is_ok());

    session.inner.reading.positions.retain_revoked(
        channel,
        generation,
        HistoryClosedReason::PermissionDenied,
    );
    assert!(session.notifications().validate_target(&target).is_err());
    client.shutdown().unwrap();
}
