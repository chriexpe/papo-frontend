//! Facts lifetime and release, tally agreement with per-row classification, and cache installs.

use super::tests::{messages, resources};
use super::*;
use crate::Generation;

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
    let mut messages = messages(user);
    Arc::make_mut(&mut messages[2]).reply_to = Some(Uuid::from_u128(10).to_string());
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
            user,
            &resources,
            range.clone(),
        );
        let tally =
            ClassificationCache::default().tally_range(channel, &messages, user, &resources, range);
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
        let tally = cache.tally_range(channel, &messages, user, resources, 0..messages.len());
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

fn plain_row(id: u128, content: &str) -> Arc<Message> {
    Arc::new(Message {
        id: Some(Uuid::from_u128(id).to_string()),
        author_id: Some(Uuid::from_u128(4).to_string()),
        content: Some(content.into()),
        ..Default::default()
    })
}

#[test]
fn a_history_full_of_ordinary_at_signs_holds_no_records_and_counts_exactly() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let mut messages: imbl::Vector<_> = (1_000..6_000)
        .map(|id| plain_row(id, "mail me at someone@example.com or ping @team"))
        .collect();
    messages.push_back(plain_row(
        9_000,
        &crate::mentions::encode_user_mention(user),
    ));
    let mut cache = ClassificationCache::default();

    let tally = cache.tally_range(channel, &messages, user, &resources(), 0..messages.len());
    assert_eq!((tally.triggered, tally.complete), (1, true));
    let facts = &cache.channels[&channel];
    assert_eq!(facts.records.len(), 1, "only the real mention is a record");
    assert_eq!(facts.classified_revisions, 1);
    assert!(!facts.has_broadcast);
}

#[test]
fn the_cache_pins_only_trigger_rows_never_the_history_it_was_given() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let mention = crate::mentions::encode_user_mention(user);
    let rows: Vec<Arc<Message>> = (0..1_000)
        .map(|id| {
            plain_row(
                100 + id,
                if id % 400 == 0 {
                    &mention
                } else {
                    "plain words"
                },
            )
        })
        .collect();
    let weak: Vec<_> = rows.iter().map(Arc::downgrade).collect();
    let mut cache = ClassificationCache::default();
    let messages: imbl::Vector<_> = rows.into_iter().collect();
    assert_eq!(
        cache
            .tally_range(channel, &messages, user, &resources(), 0..messages.len())
            .triggered,
        3
    );

    // The history is trimmed away: only the rows that count as mentions may stay reachable.
    drop(messages);
    let alive = weak.iter().filter(|row| row.upgrade().is_some()).count();
    assert_eq!(alive, 3);
}

#[test]
fn facts_are_released_for_channels_without_resident_history() {
    let user = Uuid::from_u128(1);
    let (kept, trimmed) = (Uuid::from_u128(2), Uuid::from_u128(3));
    let snapshot = snapshot(messages(user));
    let shared = std::sync::Mutex::new(ClassificationCache::default());
    for channel in [kept, trimmed] {
        ClassificationCache::project(&shared, channel, &snapshot, user, &resources(), 3..5);
    }
    assert_eq!(shared.lock().unwrap().channels.len(), 2);

    ClassificationCache::retain(&shared, &BTreeSet::from([kept]));
    let held: Vec<_> = shared.lock().unwrap().channels.keys().copied().collect();
    assert_eq!(held, [kept]);
}
