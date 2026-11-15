//! Decoding validation tests for serialized `catch_up_intervals` span JSON blobs.

use super::decode_spans;
use crate::{MessageKey, StorageError, Uuid, catch_up::ReadSpan};

fn pt(us: i64, id: u128) -> MessageKey {
    MessageKey {
        created_at_us: us,
        message_id: Uuid::from_u128(id),
    }
}

#[test]
fn decode_spans_rejects_corrupted_inverted_overlapping_or_unsorted_rows() {
    let valid = serde_json::to_vec(&vec![
        ReadSpan::normalized(pt(10, 1), pt(20, 2)),
        ReadSpan::normalized(pt(30, 3), pt(40, 4)),
    ])
    .unwrap();
    assert_eq!(decode_spans(Some(valid)).unwrap().len(), 2);

    let inverted = serde_json::to_vec(&vec![ReadSpan {
        start: pt(20, 2),
        end: pt(10, 1),
    }])
    .unwrap();
    assert!(matches!(
        decode_spans(Some(inverted)),
        Err(StorageError::InvalidState)
    ));

    let overlapping = serde_json::to_vec(&vec![
        ReadSpan::normalized(pt(10, 1), pt(25, 2)),
        ReadSpan::normalized(pt(25, 2), pt(40, 4)),
    ])
    .unwrap();
    assert!(matches!(
        decode_spans(Some(overlapping)),
        Err(StorageError::InvalidState)
    ));

    let unsorted = serde_json::to_vec(&vec![
        ReadSpan::normalized(pt(30, 3), pt(40, 4)),
        ReadSpan::normalized(pt(10, 1), pt(20, 2)),
    ])
    .unwrap();
    assert!(matches!(
        decode_spans(Some(unsorted)),
        Err(StorageError::InvalidState)
    ));
}
