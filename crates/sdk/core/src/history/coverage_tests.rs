//! Anchored coverage unit tests, cache migration checks and gap-consistency verification.
//! Restart revalidation lives in `coverage_revalidation_tests`; property oracle tests live in
//! `coverage_property_tests`.

use super::{
    Generation, HistoryOrder, HistorySnapshot,
    merge::Ticket,
    model::{Entry, Model},
    ordering::Point,
    persisted_layout::{self, Item, Metadata},
    persistence::{self, State},
    publication::Publication,
};
use crate::{
    ChronologicalKey, Message, Uuid,
    account_cache::history::{HistoryData, HistoryRow, HistoryTable},
};
use papo_api::gateway::{KnownEvent, MessageDeleteEvent};
use std::sync::Arc;

const CHANNEL_UUID_U128: u128 = 1;
pub(super) const MAX_UNIVERSE_ID: u128 = 16;

pub(super) fn channel_id() -> Uuid {
    Uuid::from_u128(CHANNEL_UUID_U128)
}

pub(super) fn key_for(id: u128) -> ChronologicalKey {
    let created_at_us = 1_700_000_000_000_000_i64 + i64::try_from(id).unwrap() * 1_000;
    ChronologicalKey {
        created_at_us,
        message_id: Uuid::from_u128(id),
    }
}

pub(super) fn message_for(id: u128) -> Message {
    let cursor = key_for(id).to_cursor().unwrap();
    Message {
        id: Some(cursor.id),
        channel_id: Some(channel_id().to_string()),
        author_id: Some(Uuid::from_u128(1000).to_string()),
        content: Some(format!("m{id}")),
        created_at: Some(cursor.created_at),
        ..Message::default()
    }
}

pub(super) fn point_for(id: u128) -> Point {
    let cursor = key_for(id).to_cursor().unwrap();
    Point {
        created_at: cursor.created_at,
        id: cursor.id,
    }
}

pub(super) fn ticket(
    model: &Model,
    cursor: Option<Point>,
    order: HistoryOrder,
    generation: u64,
    limit: usize,
) -> Ticket {
    Ticket {
        channel: *channel_id().as_bytes(),
        cursor,
        order,
        generation,
        revision: model.revision,
        limit,
    }
}

pub(super) fn snapshot_from(model: &mut Model, generation: u64, revision: u64) -> HistorySnapshot {
    let mut state = State::with_counters(Arc::default());
    std::mem::swap(&mut state.model, model);
    state.model.revision = revision;
    let mut publication = Publication::default();
    publication.publish(&state, Generation(generation)).unwrap();
    let guard = publication.snapshot.clone().unwrap();
    std::mem::swap(&mut state.model, model);
    (*guard).clone()
}

pub(super) fn encode_model_data(model: &Model) -> HistoryData {
    let metadata = persisted_layout::encode(&Metadata {
        revision: model.revision,
        empty_generation: model.empty_generation,
        active_chunks: model.chunks.iter().map(|chunk| chunk.id).collect(),
        empty_before: model.empty_before.clone(),
        empty_after: model.empty_after.clone(),
    })
    .unwrap();
    let layout = persisted_layout::layout(model).unwrap();
    let mut rows: Vec<HistoryRow> = layout
        .into_iter()
        .map(|((table_idx, id), payload)| HistoryRow {
            table: persisted_layout::partition(table_idx).unwrap(),
            id,
            payload,
        })
        .collect();
    for (id, entry) in &model.entries {
        rows.push(HistoryRow {
            table: HistoryTable::Messages,
            id: *id,
            payload: persisted_layout::encode(entry).unwrap(),
        });
    }
    HistoryData {
        set_id: 1,
        metadata,
        rows,
    }
}

fn message_entry(id: u128, revision: u64) -> Entry {
    let mut entry = Entry::empty(revision);
    entry.point = Some(point_for(id));
    entry.message = Some(message_for(id));
    entry.timeline = true;
    entry.validated_generation = 1;
    entry
}

#[test]
fn old_cache_rows_without_anchors_decode_and_round_trip() {
    let chunk_id = key_for(1).id_bytes();
    let chunk_id_json = serde_json::to_string(&chunk_id).unwrap();
    let p1_json = serde_json::to_string(&point_for(1)).unwrap();
    let p2_json = serde_json::to_string(&point_for(2)).unwrap();
    // Exact pre-13b.1 JSON payloads omitting oldest_anchor, newest_anchor, empty_before and empty_after.
    let old_metadata_json =
        format!(r#"{{"revision":4,"empty_generation":0,"active_chunks":[{chunk_id_json}]}}"#);
    let old_chunk_json = format!(
        r#"{{"chunk":{{"id":{chunk_id_json},"first":{p1_json},"last":{p2_json},"oldest_complete":true,"newest_complete":true,"generation":1,"live_generation":1}},"older":null,"newer":null}}"#
    );
    let rows = vec![
        HistoryRow {
            table: HistoryTable::Chunks,
            id: chunk_id,
            payload: old_chunk_json.into_bytes(),
        },
        HistoryRow {
            table: HistoryTable::Items,
            id: key_for(1).id_bytes(),
            payload: persisted_layout::encode(&Item::Message {
                point: point_for(1),
            })
            .unwrap(),
        },
        HistoryRow {
            table: HistoryTable::Items,
            id: key_for(2).id_bytes(),
            payload: persisted_layout::encode(&Item::Message {
                point: point_for(2),
            })
            .unwrap(),
        },
        HistoryRow {
            table: HistoryTable::Messages,
            id: key_for(1).id_bytes(),
            payload: persisted_layout::encode(&message_entry(1, 4)).unwrap(),
        },
        HistoryRow {
            table: HistoryTable::Messages,
            id: key_for(2).id_bytes(),
            payload: persisted_layout::encode(&message_entry(2, 4)).unwrap(),
        },
    ];

    let mut state = persistence::decode(HistoryData {
        set_id: 1,
        metadata: old_metadata_json.into_bytes(),
        rows,
    })
    .unwrap();
    assert!(state.model.empty_before.is_none());
    assert!(state.model.empty_after.is_none());
    assert_eq!(state.model.chunks.len(), 1);
    assert!(state.model.chunks[0].oldest_anchor.is_none());
    assert!(state.model.chunks[0].newest_anchor.is_none());
    assert_eq!(state.model.chunks[0].generation, 0);
    assert_eq!(state.model.chunks[0].live_generation, 0);

    // Re-encoding with None anchors omits all new optional fields so persisted schema stays clean.
    let reencoded = encode_model_data(&state.model);
    let meta_text = std::str::from_utf8(&reencoded.metadata).unwrap();
    assert!(
        !meta_text.contains("empty_before"),
        "unexpected empty_before in {meta_text}"
    );
    assert!(
        !meta_text.contains("empty_after"),
        "unexpected empty_after in {meta_text}"
    );
    for row in &reencoded.rows {
        let text = std::str::from_utf8(&row.payload).unwrap();
        assert!(
            !text.contains("oldest_anchor"),
            "unexpected oldest_anchor in {text}"
        );
        assert!(
            !text.contains("newest_anchor"),
            "unexpected newest_anchor in {text}"
        );
    }

    let snapshot = snapshot_from(&mut state.model, 2, 4);
    assert!(!snapshot.range(None, None).unwrap().complete);
    assert!(
        !snapshot
            .range(Some(key_for(1)), Some(key_for(2)))
            .unwrap()
            .complete
    );
}

#[test]
fn server_truth_alone_never_proves_range_complete_and_deleted_anchor_preserves_order() {
    // Server holds only message 1 and message 5; zero messages sit strictly between them.
    let mut model = Model::default();
    let t1 = ticket(&model, Some(point_for(2)), HistoryOrder::Desc, 1, 5);
    model.merge_page(&t1, &[message_for(1)], false).unwrap();
    let t2 = ticket(&model, None, HistoryOrder::Desc, 1, 1);
    model.merge_page(&t2, &[message_for(5)], true).unwrap();

    let snap = snapshot_from(&mut model, 1, 1);
    let between = snap.range(Some(key_for(1)), Some(key_for(5))).unwrap();
    assert_eq!(between.indices, 1..1);
    assert!(!between.complete);
    assert!(!snap.gaps.is_empty());

    // Fetch Asc(after = 1) -> [5]: now (1, 5) is proven complete and bridged.
    let t3 = ticket(&model, Some(point_for(1)), HistoryOrder::Asc, 1, 5);
    model.merge_page(&t3, &[message_for(5)], false).unwrap();
    let snap_bridged = snapshot_from(&mut model, 1, 2);
    let between = snap_bridged
        .range(Some(key_for(1)), Some(key_for(5)))
        .unwrap();
    assert_eq!(between.indices, 1..1);
    assert!(between.complete);
    assert!(snap_bridged.gaps.is_empty());

    // Deleted anchor keeps chronological order and never resolves to a visible message index.
    let mut anchor_model = Model::default();
    let t_asc = ticket(&anchor_model, Some(point_for(3)), HistoryOrder::Asc, 1, 2);
    anchor_model
        .merge_page(&t_asc, &[message_for(4), message_for(5)], true)
        .unwrap();
    anchor_model
        .event(
            &KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
                id: Uuid::from_u128(3).to_string(),
                channel_id: channel_id().to_string(),
            })),
            1,
        )
        .unwrap();
    let snap_deleted = snapshot_from(&mut anchor_model, 1, 1);
    let resolved = snap_deleted.resolve(key_for(3)).unwrap();
    assert!(resolved.deleted);
    assert_eq!(resolved.index, None);
    assert_eq!(resolved.key, key_for(3));
    assert!(
        snap_deleted
            .range(Some(key_for(3)), Some(key_for(5)))
            .unwrap()
            .complete
    );
}

#[test]
fn resolve_handles_legacy_millisecond_anchors_and_never_reopens_deleted() {
    let mut model = Model::default();
    let mut subms = message_for(10);
    let exact = ChronologicalKey {
        created_at_us: 1_700_000_000_010_456,
        message_id: Uuid::from_u128(10),
    };
    subms.created_at = Some(exact.to_cursor().unwrap().created_at);
    let t = ticket(&model, None, HistoryOrder::Desc, 1, 10);
    model.merge_page(&t, &[subms], false).unwrap();

    let snap = snapshot_from(&mut model, 1, 1);
    let ms_key = ChronologicalKey {
        created_at_us: 1_700_000_000_010_000,
        message_id: Uuid::from_u128(10),
    };
    let res = snap.resolve(ms_key).unwrap();
    assert!(!res.deleted);
    assert_eq!(res.index, Some(0));
    assert_eq!(res.key, exact);

    // Deleting message 10 prevents millisecond matching from returning an index.
    model
        .event(
            &KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
                id: Uuid::from_u128(10).to_string(),
                channel_id: channel_id().to_string(),
            })),
            1,
        )
        .unwrap();
    let snap_del = snapshot_from(&mut model, 1, 2);
    let res_del = snap_del.resolve(ms_key).unwrap();
    assert!(res_del.deleted);
    assert_eq!(res_del.index, None);
}

#[test]
fn unvalidated_receipt_preserves_active_empty_before_and_empty_after_proofs() {
    let mut head_model = Model::default();
    let t_empty_before = ticket(&head_model, Some(point_for(5)), HistoryOrder::Desc, 1, 10);
    head_model.merge_page(&t_empty_before, &[], false).unwrap();
    head_model.receipt(&message_for(5), 0).unwrap();
    assert!(head_model.empty_before.is_some());
    let rev_head = head_model.revision + 1;
    let snap_head = snapshot_from(&mut head_model, 1, rev_head);
    assert!(snap_head.range(None, Some(key_for(5))).unwrap().complete);
    assert!(!snap_head.gaps.iter().any(|g| g.order == HistoryOrder::Desc));

    let mut tail_model = Model::default();
    let t_empty_after = ticket(&tail_model, Some(point_for(10)), HistoryOrder::Asc, 1, 10);
    tail_model.merge_page(&t_empty_after, &[], false).unwrap();
    tail_model.receipt(&message_for(10), 0).unwrap();
    assert!(tail_model.empty_after.is_some());
    let rev_tail = tail_model.revision + 1;
    let snap_tail = snapshot_from(&mut tail_model, 1, rev_tail);
    assert!(snap_tail.range(Some(key_for(10)), None).unwrap().complete);
    assert!(!snap_tail.gaps.iter().any(|g| g.order == HistoryOrder::Asc));
}
