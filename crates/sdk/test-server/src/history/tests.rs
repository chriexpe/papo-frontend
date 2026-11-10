//! Owns deterministic history/cursor/seed tests; leaves HTTP sockets to integration tests.

use super::*;
use crate::{
    HistorySeed,
    history_seed::{insert_batch, prepare},
};
use papo_api::wire::MessageUserReaction;

const CHANNEL: &str = "10000000-0000-4000-8000-000000000001";
const AUTHOR: &str = "20000000-0000-4000-8000-000000000001";
const FIRST: &str = "70000000-0000-4000-8000-000000000001";
const AT: &str = "2026-01-01T00:00:00.123456Z";

fn recipe(count: usize) -> HistorySeed {
    HistorySeed {
        channel_id: CHANNEL.into(),
        author_id: AUTHOR.into(),
        first_message_id: FIRST.into(),
        count,
        first_created_at: AT.into(),
        timestamp_step_us: 1,
        equal_timestamp_count: 7,
    }
}

fn page(history: &History, query: &str) -> (Vec<Message>, bool) {
    history.page(CHANNEL, AUTHOR, &HistoryQuery::parse(query).ok().unwrap())
}

fn ids(messages: &[Message]) -> Vec<&str> {
    messages
        .iter()
        .map(|message| message.id.as_deref().unwrap())
        .collect()
}

#[test]
fn ten_thousand_messages_page_both_directions_without_equal_timestamp_loss() {
    let messages = recipe(10_000).messages().unwrap();
    let mut history = History::default();
    insert_batch(&mut history, messages.clone()).unwrap();
    for order in ["asc", "desc"] {
        let mut query = format!("order={order}&limit=37");
        let mut observed = Vec::new();
        loop {
            let (batch, more) = page(&history, &query);
            if let Some(last) = batch.last() {
                let cursor = last.cursor().unwrap();
                query = url::form_urlencoded::Serializer::new(String::new())
                    .append_pair("order", order)
                    .append_pair("limit", "37")
                    .append_pair("since", &cursor.created_at)
                    .append_pair("last_id", &cursor.id)
                    .finish();
            }
            observed.extend(batch.into_iter().map(|message| message.id.unwrap()));
            if !more {
                break;
            }
        }
        let mut expected: Vec<_> = messages
            .iter()
            .map(|message| message.id.clone().unwrap())
            .collect();
        if order == "desc" {
            expected.reverse();
        }
        assert_eq!(observed, expected);
    }
}

#[test]
fn exact_timestamp_strings_survive_cursor_round_trips() {
    let mut seed = recipe(3);
    seed.first_created_at = "2026-01-01T01:00:00.123456+01:00".into();
    seed.timestamp_step_us = 0;
    let mut history = History::default();
    insert_batch(&mut history, seed.messages().unwrap()).unwrap();
    let (messages, _) = page(&history, "order=asc");
    assert!(
        messages
            .iter()
            .all(|message| message.cursor().unwrap().created_at == seed.first_created_at)
    );
    let query = format!("since={AT}&last_id={FIRST}&order=asc");
    assert_eq!(page(&history, &query).0.len(), 2);
}

#[test]
fn since_only_is_forward_polling_even_when_descending() {
    let mut seed = recipe(3);
    seed.equal_timestamp_count = 1;
    let mut history = History::default();
    insert_batch(&mut history, seed.messages().unwrap()).unwrap();
    let (messages, _) = page(&history, &format!("since={AT}&order=desc"));
    assert_eq!(
        ids(&messages),
        vec![
            "70000000-0000-4000-8000-000000000003",
            "70000000-0000-4000-8000-000000000002"
        ]
    );
}

#[test]
fn last_id_without_since_is_ignored_and_default_page_is_thirty() {
    let mut history = History::default();
    insert_batch(&mut history, recipe(31).messages().unwrap()).unwrap();
    let (messages, more) = page(&history, "last_id=not-a-uuid");
    assert_eq!((messages.len(), more), (30, true));
}

#[test]
fn nano_cursor_does_not_round_to_a_microsecond_boundary() {
    let mut history = History::default();
    insert_batch(&mut history, recipe(7).messages().unwrap()).unwrap();
    assert_eq!(
        page(
            &history,
            &format!("since=2026-01-01T00:00:00.123456001Z&last_id={FIRST}&order=desc")
        )
        .0
        .len(),
        7
    );
    assert_eq!(
        page(
            &history,
            &format!("since=2026-01-01T00:00:00.123455999Z&last_id={FIRST}&order=asc")
        )
        .0
        .len(),
        7
    );
}

#[test]
fn invalid_query_values_have_pinned_backend_error_statuses() {
    for query in [
        "since=no",
        "limit=0",
        "limit=101",
        "limit=-1",
        "limit=nan",
        "order=ASC",
        "order=older",
    ] {
        assert_eq!(HistoryQuery::parse(query).err(), Some(400));
    }
    assert!(
        HistoryQuery::parse(&format!("since={AT}&last_id=no"))
            .ok()
            .unwrap()
            .invalid_last_id
    );
    assert_eq!(HistoryQuery::parse("limit=2&limit=0").unwrap().limit, 2);
}

#[test]
fn cursor_uuid_is_compared_as_bytes_and_does_not_need_to_exist() {
    let mut history = History::default();
    insert_batch(&mut history, recipe(7).messages().unwrap()).unwrap();
    let (messages, _) = page(
        &history,
        &format!("since={AT}&last_id=70000000-0000-4000-8000-000000000000&order=asc"),
    );
    assert_eq!(messages.len(), 7);
    let (messages, _) = page(
        &history,
        &format!("since={AT}&last_id=70000000-0000-4000-8000-000000000008&order=desc"),
    );
    assert_eq!(messages.len(), 7);
}

#[test]
fn exact_page_boundary_has_no_false_has_more_and_empty_page_is_empty() {
    let mut history = History::default();
    insert_batch(&mut history, recipe(7).messages().unwrap()).unwrap();
    let (messages, more) = page(&history, "limit=7");
    assert!(!more);
    let last = messages.last().unwrap().cursor().unwrap();
    let (messages, more) = page(
        &history,
        &format!("since={}&last_id={}", last.created_at, last.id),
    );
    assert_eq!((messages.len(), more), (0, false));
}

#[test]
fn own_reactions_are_account_scoped_and_shared_seed_ownership_is_discarded() {
    let mut history = History::default();
    let mut message = recipe(1).messages().unwrap().remove(0);
    message.user_reactions = Some(vec![MessageUserReaction {
        unicode: Some("x".into()),
        ..MessageUserReaction::default()
    }]);
    history.insert(message).unwrap();
    let query = HistoryQuery::parse("").ok().unwrap();
    assert!(
        history.page(CHANNEL, AUTHOR, &query).0[0]
            .user_reactions
            .as_ref()
            .unwrap()
            .is_empty()
    );
    history.set_own(
        AUTHOR,
        uuid(FIRST).unwrap(),
        vec![MessageUserReaction {
            unicode: Some("x".into()),
            ..MessageUserReaction::default()
        }],
    );
    assert_eq!(
        history.page(CHANNEL, AUTHOR, &query).0[0]
            .user_reactions
            .as_ref()
            .unwrap()
            .len(),
        1
    );
    assert!(
        history.page(CHANNEL, CHANNEL, &query).0[0]
            .user_reactions
            .as_ref()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn pins_sort_by_pin_instant_then_uuid_not_message_creation_and_keep_original_pin_time() {
    let mut history = History::default();
    let messages = recipe(3).messages().unwrap();
    insert_batch(&mut history, messages.clone()).unwrap();
    let first = uuid(messages[0].id.as_ref().unwrap()).unwrap();
    let second = uuid(messages[1].id.as_ref().unwrap()).unwrap();
    let third = uuid(messages[2].id.as_ref().unwrap()).unwrap();
    history.pin(first, Some(2)).unwrap();
    history.pin(third, Some(1)).unwrap();
    history.pin(second, Some(1)).unwrap();
    assert!(!history.pin(first, Some(0)).unwrap());
    assert_eq!(
        ids(&history.pinned(CHANNEL, AUTHOR)),
        vec![
            messages[1].id.as_deref().unwrap(),
            messages[2].id.as_deref().unwrap(),
            FIRST
        ]
    );
    history.remove(second).unwrap();
    assert_eq!(history.pinned(CHANNEL, AUTHOR).len(), 2);
    assert!(history.pin(third, None).unwrap());
    assert!(!history.pin(third, None).unwrap());
}

#[test]
fn hundred_pin_limit_is_enforced_but_existing_pin_remains_idempotent() {
    let mut history = History::default();
    let messages = recipe(101).messages().unwrap();
    insert_batch(&mut history, messages.clone()).unwrap();
    for message in &messages[..100] {
        history
            .pin(uuid(message.id.as_ref().unwrap()).unwrap(), Some(0))
            .unwrap();
    }
    assert!(
        history
            .pin(uuid(messages[100].id.as_ref().unwrap()).unwrap(), Some(0))
            .is_err()
    );
    assert!(!history.pin(uuid(FIRST).unwrap(), Some(1)).unwrap());
}

#[test]
fn invalid_or_duplicate_batch_has_no_partial_effects() {
    let mut history = History::default();
    let mut messages = recipe(3).messages().unwrap();
    messages[2].created_at = Some("invalid".into());
    assert!(insert_batch(&mut history, messages).is_err());
    assert!(page(&history, "").0.is_empty());
    let message = recipe(1).messages().unwrap().remove(0);
    assert!(insert_batch(&mut history, vec![message.clone(), message]).is_err());
    assert!(page(&history, "").0.is_empty());
}

#[test]
fn recipes_reject_overflow_invalid_ids_and_submicrosecond_seed_times() {
    for invalid in ["invalid", "2026-01-01T00:00:00.123456001Z"] {
        let mut seed = recipe(1);
        seed.first_created_at = invalid.into();
        assert!(seed.messages().is_err());
    }
    let mut seed = recipe(2);
    seed.first_message_id = Uuid::from_u128(u128::MAX).to_string();
    assert!(seed.messages().is_err());
    let mut seed = recipe(1);
    seed.equal_timestamp_count = 0;
    assert!(seed.messages().is_err());
    let mut seed = recipe(100_001);
    seed.timestamp_step_us = u64::MAX;
    assert!(seed.messages().is_err());
}

#[test]
fn configuration_recipes_create_channels_and_reject_collisions_before_binding() {
    let (resources, history) = prepare(&[recipe(10_000)]).unwrap();
    assert!(resources.channels.contains_key(CHANNEL));
    assert_eq!(history.channels[CHANNEL].len(), 10_000);
    assert!(prepare(&[recipe(1), recipe(1)]).is_err());
}

#[test]
fn denial_is_account_local_and_channel_removal_cleans_all_authoritative_state() {
    let mut history = History::default();
    insert_batch(&mut history, recipe(1).messages().unwrap()).unwrap();
    history.pin(uuid(FIRST).unwrap(), Some(0)).unwrap();
    history.set_readable(AUTHOR, CHANNEL, false);
    assert!(!history.readable(AUTHOR, CHANNEL));
    assert!(history.readable(CHANNEL, CHANNEL));
    history.remove_channel(CHANNEL);
    assert!(history.index.is_empty());
    assert!(history.pins.is_empty());
    assert!(history.readable(AUTHOR, CHANNEL));
}
