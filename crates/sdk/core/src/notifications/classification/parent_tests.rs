//! Reply-parent evidence: when a missing parent is searched again, and what stays valid.

use super::tests::{messages, resources};
use super::*;

const CHANNEL: u128 = 2;

fn row(id: u128, author: Uuid) -> Arc<Message> {
    Arc::new(Message {
        id: Some(Uuid::from_u128(id).to_string()),
        author_id: Some(author.to_string()),
        ..Default::default()
    })
}

/// The shared fixture with its reply (row 2) pointed at `parent` and 20 plain rows appended.
fn history_with_reply_to(user: Uuid, parent: Uuid) -> imbl::Vector<Arc<Message>> {
    let mut messages = messages(user);
    Arc::make_mut(&mut messages[2]).reply_to = Some(parent.to_string());
    messages.extend((100..120).map(|id| row(id, Uuid::from_u128(4))));
    messages
}

fn examined(cache: &ClassificationCache) -> usize {
    cache.channels[&Uuid::from_u128(CHANNEL)]
        .parents
        .rows_examined
}

fn classify_reply(
    cache: &mut ClassificationCache,
    messages: &imbl::Vector<Arc<Message>>,
    user: Uuid,
    index: usize,
) -> Classification {
    cache.classify_range(
        Uuid::from_u128(CHANNEL),
        messages,
        user,
        &resources(),
        index..index + 1,
    )[0]
}

#[test]
fn a_missing_parent_is_searched_once_and_tail_growth_or_edits_do_not_repeat_it() {
    let user = Uuid::from_u128(1);
    let mut messages = history_with_reply_to(user, Uuid::from_u128(99));
    let mut cache = ClassificationCache::default();

    let reply = classify_reply(&mut cache, &messages, user, 2);
    assert!(!reply.triggered && !reply.complete);
    assert_eq!(examined(&cache), messages.len());

    classify_reply(&mut cache, &messages, user, 2);
    assert_eq!(examined(&cache), messages.len(), "same snapshot");

    messages.push_back(row(200, Uuid::from_u128(4)));
    Arc::make_mut(&mut messages[10]).content = Some("edited".into());
    let reply = classify_reply(&mut cache, &messages, user, 2);
    assert!(!reply.complete);
    assert_eq!(
        examined(&cache),
        25,
        "a live append and an edit are no news"
    );

    messages.push_back(row(201, Uuid::from_u128(4)));
    classify_reply(&mut cache, &messages, user, 2);
    assert_eq!(examined(&cache), 25, "the tail may keep growing");
}

#[test]
fn a_parent_inserted_before_the_last_row_resolves_though_both_ends_are_unchanged() {
    let user = Uuid::from_u128(1);
    let parent = Uuid::from_u128(99);
    let mut messages = history_with_reply_to(user, parent);
    let mut cache = ClassificationCache::default();
    assert!(!classify_reply(&mut cache, &messages, user, 2).complete);
    let searched = examined(&cache);

    // A late row lands inside the retained window: the first and last rows are the same rows.
    messages.insert(12, row(99, user));
    let reply = classify_reply(&mut cache, &messages, user, 2);
    assert!(
        reply.triggered && reply.complete,
        "the parent is the account's"
    );
    assert!(
        examined(&cache) > searched,
        "the missing parent was searched again"
    );
}

#[test]
fn a_backfilled_parent_resolves() {
    let user = Uuid::from_u128(1);
    let mut messages = history_with_reply_to(user, Uuid::from_u128(99));
    let mut cache = ClassificationCache::default();
    assert!(!classify_reply(&mut cache, &messages, user, 2).complete);

    messages.push_front(row(99, user));
    let reply = classify_reply(&mut cache, &messages, user, 3);
    assert!(reply.triggered && reply.complete);
}

#[test]
fn a_found_parent_stays_valid_across_backfills_and_insertions() {
    let user = Uuid::from_u128(1);
    let mut messages = history_with_reply_to(user, Uuid::from_u128(118));
    Arc::make_mut(&mut messages[23]).author_id = Some(user.to_string());
    let mut cache = ClassificationCache::default();
    assert!(classify_reply(&mut cache, &messages, user, 2).triggered);
    let searched = examined(&cache);
    assert!(
        searched <= 2,
        "a recent parent is found from the newest row"
    );

    Arc::make_mut(&mut messages[2]).content = Some("edited".into());
    messages.push_back(row(200, Uuid::from_u128(4)));
    messages.push_front(row(300, Uuid::from_u128(4)));
    messages.insert(12, row(301, Uuid::from_u128(4)));
    let reply = classify_reply(&mut cache, &messages, user, 3);
    assert!(reply.triggered && reply.complete);
    assert_eq!(examined(&cache), searched, "an author never changes");
}

#[test]
fn evidence_follows_the_unread_replies() {
    let user = Uuid::from_u128(1);
    let messages = history_with_reply_to(user, Uuid::from_u128(99));
    let mut cache = ClassificationCache::default();
    classify_reply(&mut cache, &messages, user, 2);
    assert_eq!(cache.channels[&Uuid::from_u128(CHANNEL)].parents.len(), 1);

    classify_reply(&mut cache, &messages, user, 3);
    assert_eq!(
        cache.channels[&Uuid::from_u128(CHANNEL)].parents.len(),
        0,
        "the reply left the range"
    );
}
