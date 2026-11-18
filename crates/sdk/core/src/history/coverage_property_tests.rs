//! Dual-oracle property specs for anchored coverage soundness, evidence and gap consistency.
//! The oracle model lives in `coverage_property_oracle`; unit regressions live in
//! `coverage_tests` and `coverage_revalidation_tests`.

use super::{
    HistoryOrder,
    chunk_merge::LowerBound,
    coverage_property_oracle::{ORACLE_SLOTS, Oracle, check_snapshot_against_oracles},
    coverage_tests::{
        channel_id, encode_model_data, message_for, point_for, snapshot_from, ticket,
    },
    model::Model,
    persistence,
};
use crate::{Message, Uuid};
use papo_api::gateway::{KnownEvent, MessageDeleteEvent, MessageEvent};
use proptest::prelude::*;

fn live(message: &Message) -> KnownEvent {
    KnownEvent::Message(Box::new(MessageEvent {
        id: message.id.clone().unwrap(),
        channel_id: message.channel_id.clone().unwrap(),
        created_at: message.created_at.clone().unwrap(),
        ..MessageEvent::default()
    }))
}

#[derive(Clone, Debug)]
enum Op {
    FetchDesc { cursor: Option<u8>, limit: usize },
    FetchAsc { cursor: Option<u8>, limit: usize },
    LiveCreate { id: u8 },
    HiddenServerInsert { id: u8 },
    Delete { id: u8 },
    EvictPrefix,
    Reconnect,
    Restart,
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        (prop::option::of(1u8..=16), 1usize..=5)
            .prop_map(|(cursor, limit)| Op::FetchDesc { cursor, limit }),
        (prop::option::of(1u8..=16), 1usize..=5)
            .prop_map(|(cursor, limit)| Op::FetchAsc { cursor, limit }),
        (1u8..=16).prop_map(|id| Op::LiveCreate { id }),
        (1u8..=16).prop_map(|id| Op::HiddenServerInsert { id }),
        (1u8..=16).prop_map(|id| Op::Delete { id }),
        Just(Op::EvictPrefix),
        Just(Op::Reconnect),
        Just(Op::Restart),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn coverage_property_matches_dual_oracle_soundness_and_evidence(
        initial_ids in prop::collection::btree_set(1u8..=8, 0..=6),
        ops in prop::collection::vec(op_strategy(), 1..=28),
    ) {
        let mut model = Model::default();
        let mut oracle = Oracle::default();
        for id in initial_ids {
            oracle.server_live.insert(u128::from(id), message_for(u128::from(id)));
        }
        let mut gen_id = 1u64;
        let mut revision = 0u64;

        for op in ops {
            match op {
                Op::FetchDesc { cursor, limit } => {
                    let cursor_id = cursor.map(u128::from);
                    let ticket_cursor = cursor_id.map(point_for);
                    let req = ticket(&model, ticket_cursor, HistoryOrder::Desc, gen_id, limit);
                    let matching: Vec<(u128, Message)> = oracle
                        .server_live
                        .iter()
                        .rev()
                        .filter(|&(id, _)| cursor_id.is_none_or(|c| *id < c))
                        .map(|(&id, m)| (id, m.clone()))
                        .collect();
                    let has_more = matching.len() > limit;
                    let page_pairs: Vec<_> = matching.into_iter().take(limit).collect();
                    let page: Vec<Message> = page_pairs.iter().map(|(_, m)| m.clone()).collect();
                    model.merge_page(&req, &page, has_more).unwrap();

                    let lower = if has_more {
                        Oracle::slot_key(page_pairs.last().unwrap().0)
                    } else {
                        1
                    };
                    if let Some((top, _)) = page_pairs.first() {
                        let upper = cursor_id.map_or(Oracle::slot_key(*top) + 1, Oracle::slot_key);
                        if lower < upper {
                            oracle.apply_historical(lower, upper);
                        }
                        if cursor_id.is_none() {
                            oracle.set_live_tail(Oracle::slot_key(*top) + 1, gen_id);
                        }
                    } else if let Some(cid) = cursor_id {
                        let upper = Oracle::slot_key(cid);
                        if lower < upper {
                            oracle.apply_historical(lower, upper);
                        }
                    } else {
                        oracle.set_live_tail(1, gen_id);
                    }
                }
                Op::FetchAsc { cursor, limit } => {
                    let cursor_id = cursor.map(u128::from);
                    let ticket_cursor = cursor_id.map(point_for);
                    let req = ticket(&model, ticket_cursor, HistoryOrder::Asc, gen_id, limit);
                    let matching: Vec<(u128, Message)> = oracle
                        .server_live
                        .iter()
                        .filter(|&(id, _)| cursor_id.is_none_or(|c| *id > c))
                        .map(|(&id, m)| (id, m.clone()))
                        .collect();
                    let has_more = matching.len() > limit;
                    let page_pairs: Vec<_> = matching.into_iter().take(limit).collect();
                    let page: Vec<Message> = page_pairs.iter().map(|(_, m)| m.clone()).collect();
                    model.merge_page(&req, &page, has_more).unwrap();

                    let lower = cursor_id.map_or(1, |id| Oracle::slot_key(id) + 1);
                    if let Some((top, _)) = page_pairs.last() {
                        let upper = Oracle::slot_key(*top) + 1;
                        if lower < upper {
                            oracle.apply_historical(lower, upper);
                        }
                        if !has_more {
                            oracle.set_live_tail(upper, gen_id);
                        }
                    } else if !has_more {
                        oracle.set_live_tail(lower, gen_id);
                    }
                }
                Op::LiveCreate { id } => {
                    let id = u128::from(id);
                    if oracle.server_deleted.contains(&id)
                        || oracle.server_live.keys().next_back().is_some_and(|&top| id <= top)
                    {
                        continue;
                    }
                    let msg = message_for(id);
                    oracle.server_live.insert(id, msg.clone());
                    model.event(&live(&msg), gen_id).unwrap();
                    let pos = Oracle::slot_key(id);
                    if let Some(from) = oracle.live_tail_from
                        && oracle.live_gen == gen_id
                    {
                        oracle.apply_historical(from.min(pos), pos + 1);
                    } else {
                        oracle.apply_historical(pos, pos + 1);
                    }
                    oracle.live_tail_from = Some(pos + 1);
                    oracle.live_gen = gen_id;
                }
                Op::HiddenServerInsert { id } => {
                    let id = u128::from(id);
                    if oracle.server_deleted.contains(&id) || oracle.server_live.contains_key(&id) {
                        continue;
                    }
                    let slot = Oracle::slot_key(id);
                    if oracle.is_slot_covered(slot) {
                        continue;
                    }
                    oracle.server_live.insert(id, message_for(id));
                }
                Op::Delete { id } => {
                    let id = u128::from(id);
                    if oracle.server_live.remove(&id).is_some() {
                        oracle.server_deleted.insert(id);
                        model
                            .event(
                                &KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
                                    id: Uuid::from_u128(id).to_string(),
                                    channel_id: channel_id().to_string(),
                                })),
                                gen_id,
                            )
                            .unwrap();
                    }
                }
                Op::EvictPrefix => {
                    if !model.evict_prefix().unwrap().is_empty() {
                        if model.chunks.is_empty() {
                            oracle.clear_coverage();
                        } else {
                            let clear_start = model.empty_before.as_ref().map_or(1, |eb| {
                                let k = eb.point.key().unwrap();
                                Oracle::slot_key(Uuid::from_bytes(k.id_bytes()).as_u128())
                            });
                            let clear_end = match model.chunks.first().map(|c| c.lower_bound().unwrap()) {
                                None => ORACLE_SLOTS - 1,
                                Some(LowerBound::Unbounded) => 1,
                                Some(LowerBound::Included(k)) => {
                                    Oracle::slot_key(Uuid::from_bytes(k.id_bytes()).as_u128())
                                }
                                Some(LowerBound::Excluded(k)) => {
                                    Oracle::slot_key(Uuid::from_bytes(k.id_bytes()).as_u128()) + 1
                                }
                            };
                            if clear_start < clear_end {
                                for slot in &mut oracle.historical[clear_start..clear_end] {
                                    *slot = false;
                                }
                            }
                        }
                    }
                }
                Op::Reconnect => {
                    gen_id += 1;
                    oracle.clear_live(gen_id);
                }
                Op::Restart => {
                    let data = encode_model_data(&model);
                    let mut restored = persistence::decode(data).unwrap();
                    restored.model.revision = model.revision;
                    model = restored.model;
                    gen_id += 1;
                    oracle.current_gen = gen_id;
                    oracle.clear_coverage();
                }
            }
            revision += 1;
            let snapshot = snapshot_from(&mut model, gen_id, revision);
            let check = check_snapshot_against_oracles(&snapshot, &oracle);
            prop_assert!(check.is_ok(), "{}", check.unwrap_err());
        }
    }
}
