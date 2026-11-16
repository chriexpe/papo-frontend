//! Objective classification, revision reuse and incomplete authoritative evidence.

use super::*;
use crate::Generation;
use papo_api::wire::{Role, RolePermissions, RoleSummary, Server, UserSummary};

pub(super) fn messages(user: Uuid) -> imbl::Vector<Arc<Message>> {
    let message = |id: u128, author: Uuid, content: String, reply: Option<String>| {
        Arc::new(Message {
            id: Some(Uuid::from_u128(id).to_string()),
            author_id: Some(author.to_string()),
            content: Some(content),
            reply_to: reply,
            ..Default::default()
        })
    };
    let other = Uuid::from_u128(4);
    [
        message(10, user, "@everyone".into(), None),
        message(
            11,
            other,
            format!("`{}`", crate::mentions::encode_user_mention(user)),
            None,
        ),
        message(
            12,
            other,
            String::new(),
            Some(Uuid::from_u128(10).to_string()),
        ),
        message(13, other, "@everyone".into(), None),
        message(14, other, "@role(test)".into(), None),
    ]
    .into_iter()
    .collect()
}
pub(super) fn resources() -> ResourceSnapshot {
    let mut resources = ResourceSnapshot::empty(Generation(1));
    resources.server = Some(Arc::new(Server {
        owner_id: Some(Uuid::from_u128(99).to_string()),
        ..Default::default()
    }));
    resources.members = vec![UserSummary {
        id: Some(Uuid::from_u128(4).to_string()),
        roles: Some(vec![RoleSummary {
            id: Some(Uuid::from_u128(5).to_string()),
            ..Default::default()
        }]),
        ..Default::default()
    }]
    .into();
    resources.roles = vec![Role {
        id: Some(Uuid::from_u128(5).to_string()),
        permissions: Some(RolePermissions {
            everyone_message: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    }]
    .into();
    resources
}

#[test]
fn direct_reply_authorized_everyone_own_and_unsupported_roles_follow_backend_rules() {
    let user = Uuid::from_u128(1);
    let mut cache = ClassificationCache::default();
    let messages = messages(user);
    let classified = cache.classify(Uuid::from_u128(2), &messages, user, &resources());
    assert_eq!(
        classified
            .iter()
            .map(|value| value.triggered)
            .collect::<Vec<_>>(),
        vec![false, true, true, true, false]
    );
    assert!(classified.iter().all(|value| value.complete));
    let channel = cache.channels.get(&Uuid::from_u128(2)).unwrap();
    assert!(Arc::ptr_eq(
        &channel.records.get(&Uuid::from_u128(11)).unwrap().message,
        &messages[1]
    ));
}

#[test]
fn edits_and_reply_evidence_changes_reclassify_without_fetching() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let mut cache = ClassificationCache::default();
    let mut messages = messages(user);
    messages.remove(0);
    let classified = cache.classify(channel, &messages, user, &resources());
    assert!(!classified[1].triggered);
    assert!(!classified[1].complete);
    let own = Arc::new(Message {
        id: Some(Uuid::from_u128(10).to_string()),
        author_id: Some(user.to_string()),
        ..Default::default()
    });
    messages.push_front(own);
    assert!(cache.classify(channel, &messages, user, &resources())[2].triggered);
    let edited = Arc::make_mut(&mut messages[1]);
    edited.content = Some(String::new());
    edited.edited_at = Some("2026-01-01T00:00:00Z".into());
    assert!(!cache.classify(channel, &messages, user, &resources())[1].triggered);
}

#[test]
fn broadcast_rechecks_owner_roles_missing_evidence_and_webhooks_on_cached_facts() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let mut cache = ClassificationCache::default();
    let mut messages = messages(user);
    let mut resources = resources();
    assert!(cache.classify(channel, &messages, user, &resources)[3].triggered);
    Arc::make_mut(&mut resources.roles)[0].permissions = Some(RolePermissions::default());
    let denied = cache.classify(channel, &messages, user, &resources)[3];
    assert!(!denied.triggered && denied.complete);
    Arc::make_mut(&mut resources.roles)[0].permissions = None;
    let unknown = cache.classify(channel, &messages, user, &resources)[3];
    assert!(!unknown.triggered && !unknown.complete);
    resources.roles = Arc::default();
    assert!(!cache.classify(channel, &messages, user, &resources)[3].complete);
    Arc::make_mut(resources.server.as_mut().unwrap()).owner_id =
        Some(Uuid::from_u128(4).to_string());
    assert!(cache.classify(channel, &messages, user, &resources)[3].triggered);
    Arc::make_mut(&mut messages[3]).webhook_id = Some(Uuid::from_u128(6).to_string());
    let webhook = cache.classify(channel, &messages, user, &resources)[3];
    assert!(!webhook.triggered && webhook.complete);
}

#[test]
fn direct_conversations_count_foreign_messages_without_holding_facts() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let messages = messages(user);
    let mut first = ClassificationCache::default();
    let mut second = ClassificationCache::default();
    let mut resources = resources();
    resources.dms = vec![papo_api::wire::DirectConversation {
        id: channel.to_string(),
        ..Default::default()
    }]
    .into();
    let classified = first.classify(channel, &messages, user, &resources);
    assert_eq!(classified.iter().filter(|value| value.triggered).count(), 4);
    assert!(classified.iter().all(|value| value.complete));
    assert!(first.channels.is_empty(), "a DM needs no facts");
    let out_of_range = first.classify_range(channel, &messages, user, &resources, 4..7);
    assert!(out_of_range[1..].iter().all(|value| !value.complete));

    resources.dms = Arc::default();
    let other_user = Uuid::from_u128(7);
    let classified = second.classify(channel, &messages, other_user, &resources);
    assert!(!classified[1].triggered);
    assert!(!classified[2].triggered);
    assert!(second.channels.contains_key(&channel));
    second.retain_channels(&BTreeSet::new());
    assert!(!second.channels.contains_key(&channel));
}

#[test]
fn missing_parent_author_does_not_prove_reply_is_ineligible() {
    let user = Uuid::from_u128(1);
    let mut messages = messages(user);
    Arc::make_mut(&mut messages[0]).author_id = None;
    let mut cache = ClassificationCache::default();
    let classified = cache.classify(Uuid::from_u128(2), &messages, user, &resources());
    assert!(!classified[2].triggered);
    assert!(!classified[2].complete);
}

#[test]
fn empty_and_small_no_reply_ranges_never_prepare_the_retained_prefix() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let messages: imbl::Vector<_> = (100..10_100)
        .map(|id| {
            Arc::new(Message {
                id: Some(Uuid::from_u128(id).to_string()),
                author_id: Some(Uuid::from_u128(4).to_string()),
                content: Some(crate::mentions::encode_user_mention(user)),
                ..Default::default()
            })
        })
        .collect();
    let mut cache = ClassificationCache::default();
    let end = messages.len();
    assert!(
        cache
            .classify_range(channel, &messages, user, &resources(), end..end)
            .is_empty()
    );
    let facts = &cache.channels[&channel];
    assert_eq!(
        (
            facts.records.len(),
            facts.parents.len(),
            facts.classified_revisions,
            facts.parents.rows_examined
        ),
        (0, 0, 0, 0)
    );
    cache.classify_range(channel, &messages, user, &resources(), end - 2..end);
    let facts = Arc::clone(&cache.channels[&channel]);
    assert_eq!(
        (
            facts.records.len(),
            facts.parents.len(),
            facts.classified_revisions,
            facts.parents.rows_examined
        ),
        (2, 0, 2, 0)
    );
    cache.classify_range(channel, &messages, user, &resources(), end - 2..end);
    assert_eq!(
        cache.channels[&channel].classified_revisions, 2,
        "unchanged rows keep their facts"
    );
    cache.classify_range(channel, &messages, user, &resources(), end - 1..end);
    assert_eq!(cache.channels[&channel].records.len(), 1);
    assert_eq!(cache.channels[&channel].classified_revisions, 2);
    cache.classify_range(channel, &messages, user, &resources(), end - 3..end);
    assert_eq!(cache.channels[&channel].records.len(), 3);
    assert_eq!(cache.channels[&channel].parents.rows_examined, 0);
}

#[test]
fn rows_without_a_mention_marker_or_reply_build_no_facts_and_only_count_in_direct_conversations() {
    let user = Uuid::from_u128(1);
    let channel = Uuid::from_u128(2);
    let mut messages: imbl::Vector<Arc<Message>> = (100..10_100)
        .map(|id| {
            Arc::new(Message {
                id: Some(Uuid::from_u128(id).to_string()),
                author_id: Some(Uuid::from_u128(4).to_string()),
                content: Some("plain words".into()),
                ..Default::default()
            })
        })
        .collect();
    Arc::make_mut(&mut messages[50]).author_id = Some(user.to_string());
    Arc::make_mut(&mut messages[60]).content = Some("hello @everyone".into());
    let len = messages.len();
    let mut cache = ClassificationCache::default();

    let channel_rows = cache.classify_range(channel, &messages, user, &resources(), 0..len);
    let triggered: Vec<_> = (0..len)
        .filter(|index| channel_rows[*index].triggered)
        .collect();
    assert_eq!(triggered, [60], "only the authorized @everyone row counts");
    assert!(channel_rows.iter().all(|row| row.complete));

    let facts = &cache.channels[&channel];
    assert_eq!(
        facts.records.len(),
        1,
        "only the @everyone row is a candidate"
    );
    assert_eq!(facts.classified_revisions, 1);
    assert!(facts.has_broadcast);

    let mut direct = resources();
    direct.dms = vec![papo_api::wire::DirectConversation {
        id: channel.to_string(),
        ..Default::default()
    }]
    .into();
    let dm_rows = cache.classify_range(channel, &messages, user, &direct, 0..len);
    assert_eq!(dm_rows.iter().filter(|row| row.triggered).count(), len - 1);
    assert!(!dm_rows[50].triggered, "own message never counts");
    assert!(dm_rows.iter().all(|row| row.complete));
}
