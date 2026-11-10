//! Targeted in-flight page races; generated operation coverage lives in the parent tests.

use super::*;
use papo_api::{
    gateway::{MessageEmbedsUpdateEvent, ReactUpdateEvent},
    wire::{Embed, MessageReactionSummary},
};

#[test]
fn empty_head_does_not_drop_live_arrival_received_during_fetch() {
    let mut model = Model::default();
    let request = ticket(&model, None, HistoryOrder::Desc, 1, 100);
    model.event(&live(&message(0)), 1).unwrap();
    model.merge_page(&request, &[], false).unwrap();
    assert_eq!(model.order.len(), 1);
    assert_eq!(model.chunks.len(), 1);
    assert!(model.chunks[0].oldest_complete && model.chunks[0].newest_complete);
}

#[test]
fn late_page_replays_zero_reaction_and_embed_replacement_without_changing_own_reaction() {
    let mut model = Model::default();
    let request = ticket(&model, None, HistoryOrder::Desc, 1, 100);
    let id = message(0).id.unwrap();
    model
        .event(
            &KnownEvent::ReactUpdate(Box::new(ReactUpdateEvent {
                message_id: id.clone(),
                unicode: Some("x".into()),
                count: 0,
                ..ReactUpdateEvent::default()
            })),
            1,
        )
        .unwrap();
    model
        .event(
            &KnownEvent::MessageEmbedsUpdate(Box::new(MessageEmbedsUpdateEvent {
                message_id: id,
                channel_id: Uuid::from_u128(CHANNEL).to_string(),
                embeds: Vec::new(),
            })),
            1,
        )
        .unwrap();
    let mut old = message(0);
    old.embeds = Some(vec![Embed::default()]);
    old.reactions = Some(vec![MessageReactionSummary {
        unicode: Some("x".into()),
        count: Some(8),
        ..MessageReactionSummary::default()
    }]);
    old.user_reactions = Some(Vec::new());
    model.merge_page(&request, &[old], false).unwrap();
    let loaded = model.entries[Uuid::from_u128(1).as_bytes()]
        .message
        .as_ref()
        .unwrap();
    assert!(loaded.reactions.as_ref().unwrap().is_empty());
    assert!(loaded.embeds.as_ref().unwrap().is_empty());
    assert!(loaded.user_reactions.as_ref().unwrap().is_empty());
}

#[test]
fn changed_ordering_fields_reject_the_whole_page_before_mutation() {
    let mut model = Model::default();
    let request = ticket(&model, None, HistoryOrder::Desc, 1, 100);
    model.merge_page(&request, &[message(2)], true).unwrap();
    let revision = model.revision;
    let request = ticket(&model, None, HistoryOrder::Desc, 1, 100);
    let mut changed = message(2);
    changed.created_at = Some("2025-01-01T00:00:00Z".into());
    assert!(
        model
            .merge_page(&request, &[message(3), changed], false)
            .is_err()
    );
    assert_eq!(model.revision, revision);
    assert_eq!(model.entries.len(), 1);
}

#[test]
fn ascending_without_cursor_revalidates_instead_of_starting_at_a_cached_interior_run() {
    let mut model = Model::default();
    let request = ticket(&model, None, HistoryOrder::Desc, 1, 2);
    model
        .merge_page(&request, &[message(9), message(8)], true)
        .unwrap();
    assert!(!model.page(None, HistoryOrder::Asc, 2, 1).unwrap().fresh);
}

#[test]
fn equal_timestamp_live_uuid_in_a_gap_does_not_extend_the_newest_run_backwards() {
    let mut model = Model::default();
    let newest = ticket(&model, None, HistoryOrder::Desc, 1, 3);
    model
        .merge_page(&newest, &[message(9), message(8), message(7)], true)
        .unwrap();
    let older = ticket(&model, Some(point(&message(3))), HistoryOrder::Desc, 1, 3);
    model
        .merge_page(&older, &[message(2), message(1), message(0)], false)
        .unwrap();
    model.event(&live(&message(4)), 1).unwrap();
    assert_eq!(model.chunks.len(), 3);
    for pair in model.chunks.windows(2) {
        assert!(pair[0].last.key().unwrap() < pair[1].first.key().unwrap());
    }
    let before_newest = model
        .page(Some(&point(&message(7))), HistoryOrder::Desc, 100, 1)
        .unwrap();
    assert!(before_newest.messages.is_empty());
    assert!(before_newest.gap.is_some());
    assert_eq!(
        model.chunks.last().unwrap().first.id,
        message(7).id.unwrap()
    );
}

#[test]
fn a_pin_loaded_before_its_live_creation_is_promoted_without_losing_richer_payload() {
    let mut model = Model::default();
    let head = ticket(&model, None, HistoryOrder::Desc, 1, 3);
    model
        .merge_page(&head, &[message(2), message(1), message(0)], false)
        .unwrap();
    let mut pinned = message(3);
    pinned.embeds = Some(vec![Embed::default()]);
    let pins = ticket(&model, None, HistoryOrder::Desc, 1, 100);
    model.pins(&pins, &[pinned.clone()]).unwrap();
    assert!(!model.entries[Uuid::from_u128(4).as_bytes()].timeline);
    model.event(&live(&pinned), 1).unwrap();
    let entry = &model.entries[Uuid::from_u128(4).as_bytes()];
    assert!(entry.timeline);
    assert_eq!(
        entry
            .message
            .as_ref()
            .unwrap()
            .embeds
            .as_ref()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(model.chunks.len(), 1);
    assert_eq!(model.chunks[0].last.id, pinned.id.unwrap());
}

#[test]
fn a_complete_refresh_replaces_old_generation_proof_and_can_be_read_from_cache() {
    let mut model = Model::default();
    let old = ticket(&model, None, HistoryOrder::Desc, 1, 3);
    let messages = [message(2), message(1), message(0)];
    model.merge_page(&old, &messages, false).unwrap();
    model.chunks[0].generation = 0;
    let current = ticket(&model, None, HistoryOrder::Desc, 2, 3);
    model.merge_page(&current, &messages, false).unwrap();
    for direction in [HistoryOrder::Asc, HistoryOrder::Desc] {
        assert!(model.page(None, direction, 100, 2).unwrap().fresh);
    }
}

#[test]
fn pinned_snapshot_larger_than_backend_cap_is_rejected_before_mutation() {
    let mut model = Model::default();
    let request = ticket(&model, None, HistoryOrder::Desc, 1, 100);
    let messages: Vec<_> = (0..101).map(message).collect();
    assert!(model.pins(&request, &messages).is_err());
    assert_eq!(model.revision, 0);
    assert!(model.entries.is_empty());
}
