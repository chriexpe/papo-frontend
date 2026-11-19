//! Pure evidence ordering and fence regressions over real SQLite `read_points`.

use super::*;
use crate::{
    ClientError, HistoryClosedReason,
    notifications::tests::fixture,
    read_points::{ServerReadPoint, UploadedPoint},
};
use std::sync::Arc;

fn key(created_at_us: i64, id: u128) -> MessageKey {
    MessageKey {
        created_at_us,
        message_id: Uuid::from_u128(id),
    }
}

#[test]
fn ordered_and_exact_id_matrix_never_compares_without_both_keys() {
    let read_point = key(100, 10);
    assert_eq!(
        evaluate_evidence(Uuid::from_u128(5), Some(key(100, 5)), Some(read_point), &[]),
        ReadEvidence::Read
    );
    assert_eq!(
        evaluate_evidence(
            Uuid::from_u128(10),
            Some(key(100, 10)),
            Some(read_point),
            &[]
        ),
        ReadEvidence::Read
    );
    assert_eq!(
        evaluate_evidence(
            Uuid::from_u128(11),
            Some(key(100, 11)),
            Some(read_point),
            &[]
        ),
        ReadEvidence::Unread
    );
    // Existing ordered read point plus missing candidate key cannot decide read vs unread.
    assert_eq!(
        evaluate_evidence(Uuid::from_u128(11), None, Some(read_point), &[]),
        ReadEvidence::NotReady
    );
    // Exact ID match succeeds even when the candidate key is absent.
    assert_eq!(
        evaluate_evidence(
            Uuid::from_u128(77),
            None,
            Some(read_point),
            &[Some(Uuid::from_u128(77))]
        ),
        ReadEvidence::Read
    );
    // Unresolved server mark without any ordered point only suppresses its exact ID.
    assert_eq!(
        evaluate_evidence(
            Uuid::from_u128(77),
            Some(key(50, 77)),
            None,
            &[Some(Uuid::from_u128(77))]
        ),
        ReadEvidence::Read
    );
    assert_eq!(
        evaluate_evidence(
            Uuid::from_u128(78),
            Some(key(50, 78)),
            None,
            &[Some(Uuid::from_u128(77))]
        ),
        ReadEvidence::Unread
    );
}

#[test]
fn durable_points_work_with_empty_reading_image_and_ignore_recorded_at() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    client
        .block_on(async {
            let channel = Uuid::from_u128(2);
            let generation = session.generation();
            assert!(session.reading_state(channel).is_none());

            // Known point + unresolved server mark with a far-future acknowledgement time.
            // `recorded_at` must never order messages; only `known_point` and exact ID do.
            let known = key(100, 10);
            let unresolved_id = Uuid::from_u128(500);
            session
                .client
                .inner
                .registry
                .save_uploaded_read(
                    Arc::clone(&session.inner),
                    channel,
                    generation,
                    UploadedPoint {
                        server: ServerReadPoint {
                            message_id: unresolved_id,
                            recorded_at: Some("2099-01-01T00:00:00Z".into()),
                        },
                        created_at_us: None,
                        known_point: Some(known),
                    },
                )
                .await
                .unwrap();
            assert!(session.reading_state(channel).is_none());

            assert_eq!(
                message_read_evidence(
                    &session,
                    generation,
                    channel,
                    Uuid::from_u128(9),
                    Some(key(100, 9)),
                )
                .await
                .unwrap(),
                ReadEvidence::Read
            );
            assert_eq!(
                message_read_evidence(
                    &session,
                    generation,
                    channel,
                    unresolved_id,
                    Some(key(200, 500)),
                )
                .await
                .unwrap(),
                ReadEvidence::Read
            );
            assert_eq!(
                message_read_evidence(
                    &session,
                    generation,
                    channel,
                    Uuid::from_u128(11),
                    Some(key(100, 11)),
                )
                .await
                .unwrap(),
                ReadEvidence::Unread
            );
            assert_eq!(
                message_read_evidence(&session, generation, channel, Uuid::from_u128(11), None)
                    .await
                    .unwrap(),
                ReadEvidence::NotReady
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn quarantine_revocation_epoch_and_account_visibility_fences_gate_read_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    client
        .block_on(async {
            let channel = Uuid::from_u128(2);
            let generation = session.generation();
            session
                .client
                .inner
                .registry
                .advance_local_read(
                    Arc::clone(&session.inner),
                    channel,
                    generation,
                    key(100, 10),
                )
                .await
                .unwrap();

            // Quarantine yields NotReady without dropping durable points; release restores it.
            session.inner.reading.positions.quarantine(&[channel]);
            let quarantined = channel_read_evidence(&session, generation, channel)
                .await
                .unwrap();
            assert!(!quarantined.ready);
            assert!(!quarantined.boundary_known());
            assert_eq!(
                quarantined.badge_message(Uuid::from_u128(10), Some(key(100, 10))),
                ReadEvidence::NotReady
            );
            assert_eq!(
                message_read_evidence(
                    &session,
                    generation,
                    channel,
                    Uuid::from_u128(9),
                    Some(key(100, 9)),
                )
                .await
                .unwrap(),
                ReadEvidence::NotReady
            );
            session.inner.reading.positions.release(&[channel]);
            assert_eq!(
                message_read_evidence(
                    &session,
                    generation,
                    channel,
                    Uuid::from_u128(9),
                    Some(key(100, 9)),
                )
                .await
                .unwrap(),
                ReadEvidence::Read
            );

            // Revocation fence rejects reading evidence with HistoryError::Closed.
            session.inner.reading.positions.revoke(
                &[channel],
                generation,
                HistoryClosedReason::PermissionDenied,
            );
            assert!(matches!(
                message_read_evidence(
                    &session,
                    generation,
                    channel,
                    Uuid::from_u128(9),
                    Some(key(100, 9)),
                )
                .await,
                Err(ReadingError::History(HistoryError::Closed(
                    HistoryClosedReason::PermissionDenied
                )))
            ));

            // Superseded generation is rejected by the epoch fence.
            session.inner.epochs.advance().unwrap();
            assert!(matches!(
                message_read_evidence(
                    &session,
                    generation,
                    channel,
                    Uuid::from_u128(9),
                    Some(key(100, 9)),
                )
                .await,
                Err(ReadingError::Session(ClientError::Superseded))
            ));
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn hidden_or_removed_account_in_sqlite_fails_closed_instead_of_reading_unread() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    client
        .block_on(async {
            let channel = Uuid::from_u128(2);
            let generation = session.generation();
            let database = rusqlite::Connection::open(dir.path().join("state.sqlite")).unwrap();
            database
                .execute(
                    "UPDATE accounts SET hidden=1 WHERE server_id=?1 AND user_id=?2",
                    rusqlite::params![
                        session.account().server().as_bytes(),
                        session.account().user().as_bytes()
                    ],
                )
                .unwrap();
            assert!(matches!(
                message_read_evidence(
                    &session,
                    generation,
                    channel,
                    Uuid::from_u128(9),
                    Some(key(100, 9)),
                )
                .await,
                Err(ReadingError::Session(ClientError::Closed))
            ));
        })
        .unwrap();
    client.shutdown().unwrap();
}

#[test]
fn channel_gate_sync_fails_closed_under_admission_write_contention() {
    let dir = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(dir.path());
    client
        .block_on(async {
            let healthy = Uuid::from_u128(2);
            let revoked = Uuid::from_u128(3);
            let generation = session.generation();

            assert_eq!(
                channel_gate_sync(&session.inner, healthy, generation),
                ChannelGate::Proceed
            );
            assert_eq!(
                channel_gate(&session.inner, healthy, generation).await,
                ChannelGate::Proceed
            );

            // Simulate an in-progress `seal` or `deny` holding `history.admission.write()`
            // before `reading.positions` has been updated: sync callers must fail closed.
            let write_guard = session.inner.history.admission.write().await;
            assert_eq!(
                channel_gate_sync(&session.inner, healthy, generation),
                ChannelGate::Quarantined
            );

            // A revocation already recorded on `positions` still surfaces as Denied while busy.
            session.inner.reading.positions.revoke(
                &[revoked],
                generation,
                HistoryClosedReason::PermissionDenied,
            );
            assert_eq!(
                channel_gate_sync(&session.inner, revoked, generation),
                ChannelGate::Denied(HistoryClosedReason::PermissionDenied)
            );

            drop(write_guard);
            assert_eq!(
                channel_gate_sync(&session.inner, healthy, generation),
                ChannelGate::Proceed
            );
            assert_eq!(
                channel_gate_sync(&session.inner, revoked, generation),
                ChannelGate::Denied(HistoryClosedReason::PermissionDenied)
            );
            assert_eq!(
                channel_gate(&session.inner, revoked, generation).await,
                ChannelGate::Denied(HistoryClosedReason::PermissionDenied)
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}
