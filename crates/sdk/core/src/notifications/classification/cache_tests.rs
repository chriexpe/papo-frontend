//! Reply-parent evidence lifetime and release of facts for read or ineligible channels.

use super::tests::{messages, resources};
use super::*;
use crate::Generation;

fn padded_history_with_orphan_reply(user: Uuid, orphan_parent: Uuid) -> imbl::Vector<Arc<Message>> {
    let mut messages = messages(user);
    Arc::make_mut(&mut messages[2]).reply_to = Some(orphan_parent.to_string());
    // Beyond imbl's inline capacity, so that snapshot identity is observable.
    messages.extend((100..120).map(|id| {
        Arc::new(Message {
            id: Some(Uuid::from_u128(id).to_string()),
            author_id: Some(Uuid::from_u128(4).to_string()),
            ..Default::default()
        })
    }));
    messages
}

fn gap(id: u128) -> HistoryGap {
    HistoryGap {
        cursor: papo_api::HistoryCursor {
            created_at: "2026-01-01T00:00:00Z".into(),
            id: Uuid::from_u128(id).to_string(),
        },
        order: papo_api::HistoryOrder::Desc,
    }
}

fn newest_edge_gap(id: u128) -> HistoryGap {
    HistoryGap {
        order: papo_api::HistoryOrder::Asc,
        ..gap(id)
    }
}

#[test]
fn a_moving_newest_edge_gap_never_rescans_but_an_interior_gap_does() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let mut messages = padded_history_with_orphan_reply(user, Uuid::from_u128(99));
    let mut cache = ClassificationCache::default();
    let examined = |cache: &ClassificationCache| cache.channels[&channel].parents.rows_examined;
    let append = |messages: &mut imbl::Vector<Arc<Message>>, id: u128| {
        messages.push_back(Arc::new(Message {
            id: Some(Uuid::from_u128(id).to_string()),
            author_id: Some(Uuid::from_u128(4).to_string()),
            ..Default::default()
        }));
    };

    let gaps = [gap(1), newest_edge_gap(7)];
    cache.classify_range(channel, &messages, &gaps, user, &resources(), 2..3);
    let first_search = examined(&cache);
    assert_eq!(first_search, messages.len());

    // A live append moves only the newest-edge cursor.
    append(&mut messages, 500);
    let moved = [gap(1), newest_edge_gap(8)];
    cache.classify_range(channel, &messages, &moved, user, &resources(), 2..3);
    assert_eq!(examined(&cache), first_search, "tail movement is no news");

    // An interior gap changing may have supplied an older row.
    append(&mut messages, 501);
    let filled = [gap(2), newest_edge_gap(8)];
    cache.classify_range(channel, &messages, &filled, user, &resources(), 2..3);
    assert_eq!(examined(&cache), first_search + messages.len());

    // With no newest-edge gap, a lone gap that fills downward still counts.
    append(&mut messages, 502);
    let lone = [gap(3)];
    cache.classify_range(channel, &messages, &lone, user, &resources(), 2..3);
    assert_eq!(examined(&cache), first_search + 2 * messages.len() - 1);
}

#[test]
fn a_missing_parent_is_searched_again_only_after_a_backfill_or_gap_change() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let absent = Uuid::from_u128(99);
    let mut messages = padded_history_with_orphan_reply(user, absent);
    let mut cache = ClassificationCache::default();
    let examined = |cache: &ClassificationCache| cache.channels[&channel].parents.rows_examined;

    let result = cache.classify_range(channel, &messages, &[], user, &resources(), 2..3);
    assert!(!result[0].triggered && !result[0].complete);
    assert_eq!(examined(&cache), messages.len());

    cache.classify_range(channel, &messages, &[], user, &resources(), 2..3);
    assert_eq!(examined(&cache), messages.len(), "same snapshot");

    messages.push_back(Arc::new(Message {
        id: Some(Uuid::from_u128(200).to_string()),
        author_id: Some(Uuid::from_u128(4).to_string()),
        ..Default::default()
    }));
    Arc::make_mut(&mut messages[10]).content = Some("edited".into());
    let result = cache.classify_range(channel, &messages, &[], user, &resources(), 2..3);
    assert!(!result[0].complete);
    assert_eq!(examined(&cache), 25, "live append and edit: no new search");

    // A gap fill supplies rows inside the window and changes the gap set with them.
    let before = examined(&cache);
    messages.insert(
        12,
        Arc::new(Message {
            id: Some(Uuid::from_u128(201).to_string()),
            author_id: Some(Uuid::from_u128(4).to_string()),
            ..Default::default()
        }),
    );
    let gaps = [gap(7)];
    let result = cache.classify_range(channel, &messages, &gaps, user, &resources(), 2..3);
    assert!(!result[0].complete);
    assert_eq!(
        examined(&cache),
        before + messages.len(),
        "gap change: searched"
    );

    messages.push_front(Arc::new(Message {
        id: Some(absent.to_string()),
        author_id: Some(user.to_string()),
        ..Default::default()
    }));
    let result = cache.classify_range(channel, &messages, &gaps, user, &resources(), 3..4);
    assert!(
        result[0].triggered && result[0].complete,
        "backfilled parent resolves"
    );
}

#[test]
fn a_found_parent_survives_replacement_and_evidence_follows_the_unread_replies() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let parent = Uuid::from_u128(118);
    let mut messages = padded_history_with_orphan_reply(user, parent);
    Arc::make_mut(&mut messages[23]).author_id = Some(user.to_string());
    let mut cache = ClassificationCache::default();
    assert!(cache.classify_range(channel, &messages, &[], user, &resources(), 2..3)[0].triggered);
    let examined = cache.channels[&channel].parents.rows_examined;
    assert!(
        examined <= 2,
        "a recent parent is found from the newest row"
    );

    Arc::make_mut(&mut messages[2]).content = Some("edited".into());
    messages.push_back(Arc::clone(&messages[4]));
    assert!(cache.classify_range(channel, &messages, &[], user, &resources(), 2..3)[0].triggered);
    assert_eq!(cache.channels[&channel].parents.rows_examined, examined);

    // A backfill changes the first row and the gap set; evidence for a found parent must stay.
    messages.push_front(Arc::new(Message {
        id: Some(Uuid::from_u128(300).to_string()),
        author_id: Some(Uuid::from_u128(4).to_string()),
        ..Default::default()
    }));
    let gaps = [gap(7)];
    let reply = cache.classify_range(channel, &messages, &gaps, user, &resources(), 3..4);
    assert!(reply[0].triggered && reply[0].complete);
    assert_eq!(cache.channels[&channel].parents.rows_examined, examined);

    cache.classify_range(channel, &messages, &gaps, user, &resources(), 4..5);
    assert_eq!(
        cache.channels[&channel].parents.len(),
        0,
        "reply left the range"
    );
}

fn snapshot(messages: imbl::Vector<Arc<Message>>) -> HistorySnapshot {
    HistorySnapshot {
        version: 1,
        generation: Generation(1),
        messages,
        tombstones: imbl::HashSet::new(),
        chunks: Vec::new(),
        gaps: Vec::new(),
        empty_complete: false,
        server_live_complete: false,
        live_arrival_start: None,
    }
}

#[test]
fn a_read_or_ineligible_channel_releases_its_facts() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let snapshot = snapshot(messages(user));
    let shared = std::sync::Mutex::new(ClassificationCache::default());
    let len = snapshot.messages.len();
    let project = |range| {
        ClassificationCache::project(&shared, channel, &snapshot, user, &resources(), range)
    };
    let broadcast = |expected: &[Uuid]| {
        let held = shared.lock().unwrap();
        assert_eq!(
            held.broadcast_channels().into_iter().collect::<Vec<_>>(),
            expected
        );
    };

    assert_eq!(project(3..len).triggered, 1);
    broadcast(&[channel]);
    assert_eq!(project(len..len).triggered, 0);
    assert!(shared.lock().unwrap().channels.is_empty());
    broadcast(&[]);

    project(3..len);
    ClassificationCache::evict(&shared, channel);
    assert!(shared.lock().unwrap().channels.is_empty());
}

#[test]
fn the_tally_over_candidates_agrees_with_per_row_classification() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let mut messages = padded_history_with_orphan_reply(user, Uuid::from_u128(10));
    messages.push_back(Arc::new(Message {
        id: Some(Uuid::from_u128(400).to_string()),
        author_id: Some(Uuid::from_u128(4).to_string()),
        webhook_id: Some(Uuid::from_u128(5).to_string()),
        content: Some("@everyone from a webhook".into()),
        ..Default::default()
    }));
    let mut broken = Message::clone(&messages[4]);
    broken.id = Some("not-a-uuid".into());
    let broken_at = messages.len();
    messages.push_back(Arc::new(broken));
    let mut direct = resources();
    direct.dms = vec![papo_api::wire::DirectConversation {
        id: channel.to_string(),
        ..Default::default()
    }]
    .into();

    for (resources, range) in [
        (resources(), 0..messages.len()),
        (resources(), 2..4),
        (resources(), 0..broken_at),
        (resources(), 0..messages.len() + 1),
        (direct.clone(), 0..messages.len()),
        (direct, 0..messages.len() + 1),
    ] {
        let rows = ClassificationCache::default().classify_range(
            channel,
            &messages,
            &[],
            user,
            &resources,
            range.clone(),
        );
        let tally = ClassificationCache::default().tally_range(
            channel,
            &messages,
            &[],
            user,
            &resources,
            range,
        );
        assert_eq!(
            tally.triggered,
            rows.iter().filter(|row| row.triggered).count() as u64
        );
        assert_eq!(tally.complete, rows.iter().all(|row| row.complete));
    }
}

#[test]
fn a_resource_change_recounts_cached_rows_without_reclassifying_them() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let messages = messages(user);
    let mut cache = ClassificationCache::default();
    let known = |cache: &mut ClassificationCache, resources: &ResourceSnapshot| {
        let tally = cache.tally_range(channel, &messages, &[], user, resources, 0..messages.len());
        (tally.triggered, tally.complete)
    };
    let with_owner = known(&mut cache, &resources());
    let revisions = cache.channels[&channel].classified_revisions;
    assert!(revisions > 0);

    let mut without_permission = resources();
    without_permission.roles = Arc::default();
    assert_ne!(known(&mut cache, &without_permission), with_owner);
    assert_eq!(cache.channels[&channel].classified_revisions, revisions);
    assert_eq!(known(&mut cache, &resources()), with_owner);
    assert_eq!(cache.channels[&channel].classified_revisions, revisions);
}

#[test]
fn a_computation_installs_only_over_the_entry_it_started_from() {
    let channel = Uuid::from_u128(2);
    let facts = || Arc::new(ChannelFacts::default());
    let held = |shared: &std::sync::Mutex<ClassificationCache>| {
        shared.lock().unwrap().channels.get(&channel).cloned()
    };

    // Started from nothing and nothing appeared: installed.
    let shared = std::sync::Mutex::new(ClassificationCache::default());
    let first = facts();
    assert!(ClassificationCache::install(
        &shared,
        channel,
        None,
        Arc::clone(&first)
    ));
    assert!(Arc::ptr_eq(&held(&shared).unwrap(), &first));

    // Started from that entry and it is unchanged: installed over it.
    let next = facts();
    assert!(ClassificationCache::install(
        &shared,
        channel,
        Some(&first),
        Arc::clone(&next)
    ));
    assert!(Arc::ptr_eq(&held(&shared).unwrap(), &next));

    // Evicted meanwhile: older work must not bring the entry back.
    ClassificationCache::evict(&shared, channel);
    assert!(!ClassificationCache::install(
        &shared,
        channel,
        Some(&next),
        facts()
    ));
    assert!(held(&shared).is_none());

    // Replaced by a faster producer: its entry stays.
    let winner = facts();
    assert!(ClassificationCache::install(
        &shared,
        channel,
        None,
        Arc::clone(&winner)
    ));
    assert!(!ClassificationCache::install(
        &shared,
        channel,
        Some(&next),
        facts()
    ));
    assert!(Arc::ptr_eq(&held(&shared).unwrap(), &winner));

    // Started with no entry, but one appeared: it stays.
    assert!(!ClassificationCache::install(
        &shared,
        channel,
        None,
        facts()
    ));
    assert!(Arc::ptr_eq(&held(&shared).unwrap(), &winner));
}
