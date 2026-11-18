//! Regression tests for restart revalidation, partial cache merges, and reconnect live tails.
//! Core anchored unit checks live in `coverage_tests`; property tests live in `coverage_property_tests`.

use super::{
    Generation, HistoryOrder,
    coverage_tests::{encode_model_data, key_for, message_for, point_for, snapshot_from, ticket},
    model::Model,
    persistence,
};
use papo_api::gateway::{KnownEvent, MessageDeleteEvent, MessageEvent};

fn live_create(id: u128) -> KnownEvent {
    let msg = message_for(id);
    KnownEvent::Message(Box::new(MessageEvent {
        id: msg.id.unwrap(),
        channel_id: msg.channel_id.unwrap(),
        created_at: msg.created_at.unwrap(),
        ..MessageEvent::default()
    }))
}

fn delete_local(model: &mut Model, id: u128, gen_id: u64) {
    let msg = message_for(id);
    model
        .event(
            &KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
                id: msg.id.unwrap(),
                channel_id: msg.channel_id.unwrap(),
            })),
            gen_id,
        )
        .unwrap();
}

#[test]
fn restarted_anchored_coverage_requires_revalidation_before_complete() {
    let mut model = Model::default();
    let asc_ticket = ticket(&model, Some(point_for(3)), HistoryOrder::Asc, 1, 2);
    model
        .merge_page(&asc_ticket, &[message_for(6), message_for(7)], true)
        .unwrap();
    let empty_old = ticket(&model, Some(point_for(2)), HistoryOrder::Desc, 1, 10);
    model.merge_page(&empty_old, &[], false).unwrap();
    let empty_new = ticket(&model, Some(point_for(8)), HistoryOrder::Asc, 1, 10);
    model.merge_page(&empty_new, &[], false).unwrap();

    let snap_gen1 = snapshot_from(&mut model, 1, 1);
    assert!(
        snap_gen1
            .range(Some(key_for(3)), Some(key_for(7)))
            .unwrap()
            .complete
    );
    assert!(snap_gen1.range(None, Some(key_for(2))).unwrap().complete);
    assert!(snap_gen1.range(Some(key_for(8)), None).unwrap().complete);
    assert!(
        !snap_gen1
            .range(Some(key_for(2)), Some(key_for(3)))
            .unwrap()
            .complete
    );

    // Simulate process restart: encode rows, decode into fresh State and publish under Generation 2.
    let data = encode_model_data(&model);
    let mut restarted = persistence::decode(data).unwrap();
    assert!(restarted.model.chunks[0].oldest_anchor.is_some());
    assert!(restarted.model.empty_before.is_some());
    assert!(restarted.model.empty_after.is_some());
    assert_eq!(restarted.model.chunks[0].generation, 0);

    let rev0 = restarted.model.revision;
    let snap_restarted = snapshot_from(&mut restarted.model, 2, rev0);
    assert!(
        !snap_restarted
            .range(Some(key_for(3)), Some(key_for(7)))
            .unwrap()
            .complete
    );
    assert!(
        !snap_restarted
            .range(None, Some(key_for(2)))
            .unwrap()
            .complete
    );
    assert!(
        !snap_restarted
            .range(Some(key_for(8)), None)
            .unwrap()
            .complete
    );

    // Revalidate in Generation 2.
    let rev_asc = ticket(
        &restarted.model,
        Some(point_for(3)),
        HistoryOrder::Asc,
        2,
        2,
    );
    restarted
        .model
        .merge_page(&rev_asc, &[message_for(6), message_for(7)], true)
        .unwrap();
    let rev_old = ticket(
        &restarted.model,
        Some(point_for(2)),
        HistoryOrder::Desc,
        2,
        10,
    );
    restarted.model.merge_page(&rev_old, &[], false).unwrap();
    let rev_new = ticket(
        &restarted.model,
        Some(point_for(8)),
        HistoryOrder::Asc,
        2,
        10,
    );
    restarted.model.merge_page(&rev_new, &[], false).unwrap();

    let rev = restarted.model.revision + 1;
    let snap_revalidated = snapshot_from(&mut restarted.model, 2, rev);
    assert!(
        snap_revalidated
            .range(Some(key_for(3)), Some(key_for(7)))
            .unwrap()
            .complete
    );
    assert!(
        snap_revalidated
            .range(None, Some(key_for(2)))
            .unwrap()
            .complete
    );
    assert!(
        snap_revalidated
            .range(Some(key_for(8)), None)
            .unwrap()
            .complete
    );
}

#[test]
fn partial_revalidation_across_multiple_asc_pages_merges_without_poisoning_generation() {
    let mut model = Model::default();
    let initial: Vec<_> = (1..=8).rev().map(message_for).collect();
    let t_init = ticket(&model, None, HistoryOrder::Desc, 1, 10);
    model.merge_page(&t_init, &initial, false).unwrap();

    // Restart with `[1..=8]` cached (`generation == 0`); simulate message `1` deleted offline.
    let data = encode_model_data(&model);
    let mut restarted = persistence::decode(data).unwrap();
    assert_eq!(restarted.model.chunks.len(), 1);
    assert_eq!(restarted.model.chunks[0].generation, 0);

    // Page 1 (`Asc(None, limit = 4)`) returns `[2..=5]` and proves `[2..=5]` in gen 2 while leaving `[6..=8]` unvalidated.
    let t_asc1 = ticket(&restarted.model, None, HistoryOrder::Asc, 2, 4);
    let p1: Vec<_> = (2..=5).map(message_for).collect();
    restarted.model.merge_page(&t_asc1, &p1, true).unwrap();

    let rev = restarted.model.revision + 1;
    let snap_mid = snapshot_from(&mut restarted.model, 2, rev);
    assert_eq!(snap_mid.messages.len(), 7);
    assert_eq!(snap_mid.chunks.len(), 2);
    assert_eq!(snap_mid.chunks[0].generation, Some(Generation(2)));
    assert_eq!(snap_mid.chunks[1].generation, None);
    assert!(snap_mid.range(None, Some(key_for(5))).unwrap().complete);
    assert!(
        !snap_mid
            .range(Some(key_for(5)), Some(key_for(6)))
            .unwrap()
            .complete
    );
    assert!(!snap_mid.gaps.is_empty());

    // Page 2 (`Asc(5, limit = 10)`) returns `[6..=8], has_more = false` -> merges cleanly back into one gen-2 chunk.
    let t_asc2 = ticket(
        &restarted.model,
        Some(point_for(5)),
        HistoryOrder::Asc,
        2,
        10,
    );
    let p2: Vec<_> = (6..=8).map(message_for).collect();
    restarted.model.merge_page(&t_asc2, &p2, false).unwrap();

    let rev = restarted.model.revision + 1;
    let snap_full = snapshot_from(&mut restarted.model, 2, rev);
    assert_eq!(snap_full.chunks.len(), 1);
    assert_eq!(snap_full.chunks[0].generation, Some(Generation(2)));
    assert_eq!(snap_full.chunks[0].live_generation, Some(Generation(2)));
    assert!(snap_full.chunks[0].oldest_complete);
    assert!(snap_full.chunks[0].newest_complete);
    assert!(snap_full.gaps.is_empty());
    assert!(snap_full.range(None, None).unwrap().complete);
}

#[test]
fn stale_live_tail_from_prior_generation_never_bridges_across_reconnect() {
    let mut model = Model::default();
    model.event(&live_create(1), 1).unwrap();

    // Generation 2 reconnects after hidden server inserts `2` and `3`, then fetches newest page `[3]`.
    let t_desc = ticket(&model, None, HistoryOrder::Desc, 2, 1);
    model.merge_page(&t_desc, &[message_for(3)], true).unwrap();

    let rev = model.revision + 1;
    let snap = snapshot_from(&mut model, 2, rev);
    assert_eq!(snap.chunks.len(), 2);
    assert!(!snap.chunks[0].newest_complete);
    assert!(
        !snap
            .range(Some(key_for(1)), Some(key_for(2)))
            .unwrap()
            .complete
    );
    assert!(
        !snap
            .range(Some(key_for(2)), Some(key_for(3)))
            .unwrap()
            .complete
    );
    assert!(
        !snap
            .range(Some(key_for(1)), Some(key_for(3)))
            .unwrap()
            .complete
    );
    assert_eq!(snap.gaps.len(), 3);
}

#[test]
fn deleted_live_tail_tombstone_never_extends_historical_coverage_across_reconnect() {
    let mut model = Model::default();
    model.event(&live_create(16), 1).unwrap();
    delete_local(&mut model, 16, 1);

    // `Desc(None)` sees only `[1]` at gen 1's live tail; omitted deleted `16` must not widen `last` to `16`.
    let t_desc = ticket(&model, None, HistoryOrder::Desc, 1, 1);
    model.merge_page(&t_desc, &[message_for(1)], false).unwrap();

    let rev = model.revision + 1;
    let snap_gen2 = snapshot_from(&mut model, 2, rev);
    assert!(!snap_gen2.range(None, Some(key_for(2))).unwrap().complete);
}

#[test]
fn empty_pages_inside_unvalidated_restored_chunk_preserve_range_and_gaps_agreement() {
    let mut model = Model::default();
    let p_init = ticket(&model, Some(point_for(1)), HistoryOrder::Asc, 1, 3);
    model
        .merge_page(
            &p_init,
            &[message_for(2), message_for(3), message_for(4)],
            true,
        )
        .unwrap();
    let mut restarted = persistence::decode(encode_model_data(&model)).unwrap();

    // `Desc(2) -> []` trims `[2..=4]` head without losing `empty_before` under `generation == 0`.
    let t_desc = ticket(
        &restarted.model,
        Some(point_for(2)),
        HistoryOrder::Desc,
        2,
        10,
    );
    restarted.model.merge_page(&t_desc, &[], false).unwrap();
    let rev_desc = restarted.model.revision + 1;
    let snap_desc = snapshot_from(&mut restarted.model, 2, rev_desc);
    assert!(snap_desc.range(None, Some(key_for(2))).unwrap().complete);
    assert!(!snap_desc.range(None, Some(key_for(3))).unwrap().complete);
    assert!(!snap_desc.gaps.iter().any(|g| g.order == HistoryOrder::Desc));

    // Delete 4 locally then query `Asc(4) -> []` (live_key 3 < cursor 4) inside `gen == 0` chunk.
    delete_local(&mut restarted.model, 4, 2);
    let t_asc_miss = ticket(
        &restarted.model,
        Some(point_for(4)),
        HistoryOrder::Asc,
        2,
        10,
    );
    restarted.model.merge_page(&t_asc_miss, &[], false).unwrap();
    let rev_miss = restarted.model.revision + 1;
    let snap_miss = snapshot_from(&mut restarted.model, 2, rev_miss);
    assert!(snap_miss.range(Some(key_for(4)), None).unwrap().complete);
    assert!(!snap_miss.range(Some(key_for(3)), None).unwrap().complete);
    assert!(snap_miss.gaps.iter().any(|g| g.order == HistoryOrder::Asc));

    // `Asc(3) -> []` at the newest live boundary trims `last` to 3 and clears the `Asc` gap.
    let t_asc_hit = ticket(
        &restarted.model,
        Some(point_for(3)),
        HistoryOrder::Asc,
        2,
        10,
    );
    restarted.model.merge_page(&t_asc_hit, &[], false).unwrap();
    let rev_hit = restarted.model.revision + 1;
    let snap_hit = snapshot_from(&mut restarted.model, 2, rev_hit);
    assert!(snap_hit.range(Some(key_for(3)), None).unwrap().complete);
    assert!(snap_hit.gaps.is_empty());
}

#[test]
fn unvalidated_touching_chunk_and_page_target_keep_seam_and_anchored_neighbors() {
    let mut model = Model::default();
    let p_left = ticket(&model, None, HistoryOrder::Asc, 1, 4);
    model
        .merge_page(
            &p_left,
            &[
                message_for(1),
                message_for(2),
                message_for(3),
                message_for(4),
            ],
            true,
        )
        .unwrap();
    delete_local(&mut model, 4, 1);
    let mut restarted = persistence::decode(encode_model_data(&model)).unwrap();

    // `Asc(4) -> [6, 7]` touches unvalidated `[1..=4]` at deleted tombstone 4 without shrinking `last`.
    let p_right = ticket(
        &restarted.model,
        Some(point_for(4)),
        HistoryOrder::Asc,
        2,
        2,
    );
    restarted
        .model
        .merge_page(&p_right, &[message_for(6), message_for(7)], true)
        .unwrap();
    let rev = restarted.model.revision + 1;
    let snap = snapshot_from(&mut restarted.model, 2, rev);
    assert_eq!(snap.chunks.len(), 2);
    assert!(
        snap.range(Some(key_for(4)), Some(key_for(7)))
            .unwrap()
            .complete
    );
    assert_eq!(snap.gaps.len(), 1);
    assert_eq!(snap.gaps[0].order, HistoryOrder::Asc);

    // Querying `Asc(4)` from cache prefers anchored neighbor `[6..=7]` over `C0` ending at 4.
    let cached_asc = restarted
        .model
        .page(Some(&point_for(4)), HistoryOrder::Asc, 1, 2)
        .unwrap();
    assert!(cached_asc.fresh);
    assert_eq!(cached_asc.messages.len(), 1);
    let cached_asc_all = restarted
        .model
        .page(Some(&point_for(4)), HistoryOrder::Asc, 10, 2)
        .unwrap();
    assert_eq!(cached_asc_all.messages.len(), 2);

    // Symmetric `Desc`: gen-1 `[1..=3]` (`newest_anchor = Some(6)`) + gen-0 `[6..=8]` serves `[1..=3]`.
    let mut sym = Model::default();
    let sym_right = ticket(&sym, Some(point_for(5)), HistoryOrder::Asc, 1, 3);
    sym.merge_page(
        &sym_right,
        &[message_for(6), message_for(7), message_for(8)],
        false,
    )
    .unwrap();
    let mut sym_restarted = persistence::decode(encode_model_data(&sym)).unwrap();
    let sym_left = ticket(
        &sym_restarted.model,
        Some(point_for(6)),
        HistoryOrder::Desc,
        2,
        3,
    );
    sym_restarted
        .model
        .merge_page(
            &sym_left,
            &[message_for(3), message_for(2), message_for(1)],
            false,
        )
        .unwrap();
    let cached_desc = sym_restarted
        .model
        .page(Some(&point_for(6)), HistoryOrder::Desc, 10, 2)
        .unwrap();
    assert!(cached_desc.fresh);
    assert_eq!(cached_desc.messages.len(), 3);
}
