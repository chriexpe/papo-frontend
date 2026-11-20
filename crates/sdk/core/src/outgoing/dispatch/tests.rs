//! Pure admission tests leave sockets and clocks to integration proofs.

use super::*;
use crate::{OperationKind, OutgoingMessage};

#[test]
fn unresolved_channel_head_blocks_followers_but_not_another_channel() {
    let make = |id, channel, state| {
        Arc::new(Operation {
            id: Uuid::from_u128(id),
            channel_id: Uuid::from_u128(channel),
            kind: Arc::new(OperationKind::Message(OutgoingMessage::default())),
            state,
            attempts: 0,
            depends_on: None,
            failure: None,
            message_id: None,
            message: None,
            history_projected: true,
            created_at_us: 0,
        })
    };
    let mut snapshot = OutgoingSnapshot {
        account: None,
        version: 0,
        upload_progress: None,
        fault: None,
        operations: vec![
            make(1, 1, OperationState::Unknown),
            make(2, 1, OperationState::Queued),
            make(3, 2, OperationState::Queued),
        ],
    };
    assert_eq!(next(&snapshot).unwrap().id, Uuid::from_u128(3));
    snapshot.operations.pop();
    assert!(next(&snapshot).is_none());
}

#[test]
fn classify_returns_queued_only_for_proven_not_dispatched_and_unknown_for_ambiguous_failures() {
    let (state, outcome, failure) = classify(Err(papo_api::Error::NotDispatched));
    assert_eq!(state, OperationState::Queued);
    assert!(outcome.is_none());
    assert!(failure.is_none());

    for ambiguous in [
        papo_api::Error::Transport,
        papo_api::Error::Decode,
        papo_api::Error::Http { status: 408 },
        papo_api::Error::Http { status: 500 },
        papo_api::Error::Http { status: 503 },
    ] {
        let (state, outcome, failure) = classify(Err(ambiguous));
        assert_eq!(state, OperationState::Unknown);
        assert!(outcome.is_none());
        assert!(failure.is_none());
    }

    let (state, outcome, failure) = classify(Err(papo_api::Error::Http { status: 403 }));
    assert_eq!(state, OperationState::Failed);
    assert!(outcome.is_none());
    assert_eq!(failure, Some(OperationFailure::Rejected(403)));
}
