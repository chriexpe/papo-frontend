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
    let (state, outcome, failure) =
        classify(Attempt::Answered(Err(papo_api::Error::NotDispatched)));
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
        let (state, outcome, failure) = classify(Attempt::Answered(Err(ambiguous)));
        assert_eq!(state, OperationState::Unknown);
        assert!(outcome.is_none());
        assert!(failure.is_none());
    }

    let (state, outcome, failure) = classify(Attempt::Answered(Err(papo_api::Error::Http {
        status: 403,
    })));
    assert_eq!(state, OperationState::Failed);
    assert!(outcome.is_none());
    assert_eq!(failure, Some(OperationFailure::Rejected(403)));
}

#[test]
fn an_unresolved_reaction_does_not_hold_its_channel() {
    let make = |id, kind, state| {
        Arc::new(Operation {
            id: Uuid::from_u128(id),
            channel_id: Uuid::from_u128(1),
            kind: Arc::new(kind),
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
    let reaction = || OperationKind::Reaction {
        target: MessageTarget::Remote(Uuid::from_u128(9)),
        emoji_id: None,
        unicode: Some("👍".into()),
        add: true,
    };
    let message = || OperationKind::Message(OutgoingMessage::default());
    let mut snapshot = OutgoingSnapshot {
        account: None,
        version: 0,
        upload_progress: None,
        fault: None,
        operations: vec![
            make(1, reaction(), OperationState::Failed),
            make(2, reaction(), OperationState::Unknown),
            make(3, message(), OperationState::Queued),
        ],
    };
    assert_eq!(next(&snapshot).unwrap().id, Uuid::from_u128(3));
    // An unresolved message still holds the reactions behind it.
    snapshot.operations = vec![
        make(1, message(), OperationState::Failed),
        make(2, reaction(), OperationState::Queued),
    ];
    assert!(next(&snapshot).is_none());
}

#[test]
fn a_cancel_before_the_last_byte_fails_and_after_it_stays_unknown() {
    let (state, _, failure) = classify(Attempt::Cancelled {
        body_complete: false,
    });
    assert_eq!(state, OperationState::Failed);
    assert_eq!(failure, Some(OperationFailure::Cancelled));
    let (state, _, failure) = classify(Attempt::Cancelled {
        body_complete: true,
    });
    assert_eq!(state, OperationState::Unknown);
    assert_eq!(failure, None);
}
