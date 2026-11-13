//! Canonical timestamp retention across clock skew, source differences and wall-clock rollback.
use super::{ledger, tests::fixture};
use crate::Uuid;
#[test]
fn server_time_ahead_of_local_delivery_does_not_expire_identity_before_admission() {
    let directory = tempfile::tempdir().unwrap();
    let (mut client, session) = fixture(directory.path());
    client
        .block_on(async {
            let generation = session.generation();
            let id = Uuid::from_u128(3);
            assert!(
                ledger::claim_candidate(&session, generation, id, 100, 0)
                    .await
                    .unwrap()
            );
            let thirty_days_us = 30 * 24 * 60 * 60 * 1_000_000_i64;
            assert!(
                ledger::claim_candidate(
                    &session,
                    generation,
                    Uuid::from_u128(4),
                    thirty_days_us + 1,
                    thirty_days_us + 1
                )
                .await
                .unwrap()
            );
            assert!(
                !ledger::claim_candidate(&session, generation, id, 100, thirty_days_us + 1)
                    .await
                    .unwrap()
            );
            assert!(
                ledger::claim_candidate(
                    &session,
                    generation,
                    Uuid::from_u128(5),
                    thirty_days_us + 101,
                    thirty_days_us + 101
                )
                .await
                .unwrap()
            );
            assert!(
                !ledger::claim_candidate(&session, generation, id, 100, thirty_days_us + 101)
                    .await
                    .unwrap()
            );
            assert!(
                !ledger::claim_candidate(&session, generation, id, 100, 0)
                    .await
                    .unwrap()
            );
        })
        .unwrap();
    client.shutdown().unwrap();
}
