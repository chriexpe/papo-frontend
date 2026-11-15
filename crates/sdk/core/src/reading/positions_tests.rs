//! Deterministic unit specs for reading-position coalescing and revocation-epoch fencing.
//! Coordinator queueing and batch flushing live in `reading::positions`.

use super::{PendingChannelWrite, PositionCoordinator};
use crate::{
    Generation, HistoryClosedReason, HistoryError, ReadingError, catch_up::ReadingPosition,
};
use papo_api::Uuid;
use std::collections::BTreeSet;
use tokio::sync::oneshot;

#[test]
fn coalesced_waiters_receive_concrete_revocation_reason_on_revoke() {
    let coordinator = PositionCoordinator::default();
    let channel = Uuid::from_u128(0x1111_0000_4000_8000_0000_0000_0000_0001);
    let message_id = Uuid::from_u128(0x7111_0000_4000_8000_0000_0000_0000_0001);
    let generation = Generation(3);
    let mut receivers = Vec::new();
    {
        let mut state = coordinator
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = state.pending.entry(channel).or_insert(PendingChannelWrite {
            generation,
            revocation_epoch: 0,
            position: Some(ReadingPosition {
                message_id,
                offset_units: -7,
            }),
            waiters: Vec::new(),
        });
        for _ in 0..4 {
            let (tx, rx) = oneshot::channel();
            entry.waiters.push(tx);
            receivers.push(rx);
        }
    }

    coordinator.revoke(&[channel], generation, HistoryClosedReason::ChannelDeleted);

    for mut rx in receivers {
        let outcome = rx
            .try_recv()
            .expect("waiter must be notified synchronously");
        assert!(matches!(
            outcome,
            Err(ReadingError::History(HistoryError::Closed(
                HistoryClosedReason::ChannelDeleted
            )))
        ));
    }
}

#[test]
fn stale_revocation_epoch_remains_fenced_after_same_generation_regrant() {
    let coordinator = PositionCoordinator::default();
    let channel = Uuid::from_u128(0x1111_0000_4000_8000_0000_0000_0000_0002);
    let generation = Generation(5);
    assert_eq!(coordinator.check_writable(channel, generation, 0), None);

    coordinator.revoke(&[channel], generation, HistoryClosedReason::ChannelDeleted);
    assert_eq!(
        coordinator.check_writable(channel, generation, 0),
        Some(HistoryClosedReason::ChannelDeleted)
    );
    assert_eq!(
        coordinator.check_writable(channel, generation, 1),
        Some(HistoryClosedReason::ChannelDeleted)
    );

    let visible = BTreeSet::from([channel]);
    coordinator.clear_revocations(&visible, generation);
    // Current re-granted epoch becomes writable, while pre-revocation epoch stays fenced.
    assert_eq!(coordinator.check_writable(channel, generation, 1), None);
    assert_eq!(
        coordinator.check_writable(channel, generation, 0),
        Some(HistoryClosedReason::ChannelDeleted)
    );
}
