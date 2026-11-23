//! This account's reaction membership, reaction totals and pin order against the history model:
//! what an older page may not undo, and what a newer one replaces.

use papo_api::{
    HistoryOrder, Uuid,
    gateway::{KnownEvent, MessageDeleteEvent, MessagePinEvent, ReactUpdateEvent},
    wire::{Message, MessageReactionSummary, MessageUserReaction},
};

use super::{merge::Ticket, model::Model, own_reactions::OwnReaction};

const CHANNEL: u128 = 810_000;
const GENERATION: u64 = 1;

fn message(index: u128) -> Message {
    Message {
        id: Some(Uuid::from_u128(index + 1).to_string()),
        channel_id: Some(Uuid::from_u128(CHANNEL).to_string()),
        created_at: Some(format!("2026-01-01T00:00:{:02}.000000Z", index % 60)),
        content: Some(String::new()),
        ..Message::default()
    }
}

fn id(index: u128) -> [u8; 16] {
    *Uuid::from_u128(index + 1).as_bytes()
}

fn ticket(model: &Model) -> Ticket {
    Ticket {
        channel: *Uuid::from_u128(CHANNEL).as_bytes(),
        generation: GENERATION,
        revision: model.revision,
        cursor: None,
        order: HistoryOrder::Desc,
        limit: 100,
    }
}

fn thumbs(add: bool) -> OwnReaction {
    OwnReaction {
        emoji_id: None,
        unicode: Some("👍".into()),
        add,
    }
}

fn mine(model: &Model, index: u128) -> Vec<String> {
    model.entries[&id(index)]
        .message
        .as_ref()
        .and_then(|message| message.user_reactions.clone())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|reaction| reaction.unicode)
        .collect()
}

fn with_mine(mut message: Message, unicode: &[&str]) -> Message {
    message.user_reactions = Some(
        unicode
            .iter()
            .map(|unicode| MessageUserReaction {
                id: None,
                emoji_id: None,
                unicode: Some((*unicode).into()),
            })
            .collect(),
    );
    message
}

fn total(unicode: &str, count: i64) -> MessageReactionSummary {
    MessageReactionSummary {
        emoji_id: None,
        unicode: Some(unicode.into()),
        count: Some(count),
    }
}

fn react(index: u128, unicode: &str, count: i64) -> KnownEvent {
    KnownEvent::ReactUpdate(Box::new(ReactUpdateEvent {
        message_id: Uuid::from_u128(index + 1).to_string(),
        emoji_id: None,
        unicode: Some(unicode.into()),
        count,
    }))
}

fn pin(index: u128, is_pinned: bool) -> KnownEvent {
    KnownEvent::MessagePin(Box::new(MessagePinEvent {
        message_id: Uuid::from_u128(index + 1).to_string(),
        is_pinned,
    }))
}

#[test]
fn a_confirmed_reaction_survives_a_page_fetched_before_it() {
    let mut model = Model::default();
    model
        .merge_page(&ticket(&model), &[message(0)], false)
        .unwrap();
    let before = ticket(&model);
    assert!(model.own_reaction(id(0), thumbs(true)).unwrap());
    assert_eq!(mine(&model, 0), ["👍"]);
    model.merge_page(&before, &[message(0)], false).unwrap();
    assert_eq!(mine(&model, 0), ["👍"]);
}

#[test]
fn a_page_fetched_after_a_confirmed_reaction_is_the_truth() {
    let mut model = Model::default();
    model
        .merge_page(&ticket(&model), &[message(0)], false)
        .unwrap();
    model.own_reaction(id(0), thumbs(true)).unwrap();
    // Removed on another device since: the newer page says so.
    model
        .merge_page(&ticket(&model), &[message(0)], false)
        .unwrap();
    assert_eq!(mine(&model, 0), Vec::<String>::new());
}

#[test]
fn a_confirmed_removal_survives_a_page_fetched_before_it() {
    let mut model = Model::default();
    let held = with_mine(message(0), &["👍", "🔥"]);
    model
        .merge_page(&ticket(&model), std::slice::from_ref(&held), false)
        .unwrap();
    let before = ticket(&model);
    model.own_reaction(id(0), thumbs(false)).unwrap();
    assert_eq!(mine(&model, 0), ["🔥"]);
    model.merge_page(&before, &[held], false).unwrap();
    assert_eq!(mine(&model, 0), ["🔥"]);
}

#[test]
fn settling_the_same_reaction_twice_reaches_the_same_state() {
    let mut model = Model::default();
    model
        .merge_page(&ticket(&model), &[message(0)], false)
        .unwrap();
    model.own_reaction(id(0), thumbs(true)).unwrap();
    model.own_reaction(id(0), thumbs(true)).unwrap();
    assert_eq!(mine(&model, 0), ["👍"]);
    assert_eq!(model.entries[&id(0)].own_reactions.len(), 1);
}

#[test]
fn a_deleted_message_takes_no_reaction() {
    let mut model = Model::default();
    model
        .merge_page(&ticket(&model), &[message(0)], false)
        .unwrap();
    model
        .event(
            &KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
                id: Uuid::from_u128(1).to_string(),
                channel_id: Uuid::from_u128(CHANNEL).to_string(),
            })),
            GENERATION,
        )
        .unwrap();
    assert!(!model.own_reaction(id(0), thumbs(true)).unwrap());
    assert_eq!(model.entries[&id(0)].own_reactions, []);
}

#[test]
fn a_reaction_names_exactly_one_emoji() {
    let mut model = Model::default();
    let both = OwnReaction {
        emoji_id: Some(Uuid::from_u128(9).to_string()),
        unicode: Some("👍".into()),
        add: true,
    };
    assert!(model.own_reaction(id(0), both).is_err());
}

#[test]
fn a_changed_total_keeps_its_place_and_zero_removes_it() {
    let mut model = Model::default();
    let mut held = message(0);
    held.reactions = Some(vec![total("🧀", 1), total("👍", 2)]);
    model.merge_page(&ticket(&model), &[held], false).unwrap();
    let totals = |model: &Model| {
        model.entries[&id(0)]
            .message
            .as_ref()
            .unwrap()
            .reactions
            .clone()
            .unwrap()
            .into_iter()
            .map(|total| (total.unicode.unwrap(), total.count.unwrap()))
            .collect::<Vec<_>>()
    };
    model.event(&react(0, "🧀", 3), GENERATION).unwrap();
    assert_eq!(totals(&model), [("🧀".into(), 3), ("👍".into(), 2)]);
    model.event(&react(0, "🔥", 1), GENERATION).unwrap();
    model.event(&react(0, "🧀", 0), GENERATION).unwrap();
    assert_eq!(totals(&model), [("👍".into(), 2), ("🔥".into(), 1)]);
}

#[test]
fn pins_are_newest_first_and_current_only_once_fetched() {
    let mut model = Model::default();
    model
        .merge_page(&ticket(&model), &[message(5), message(4)], false)
        .unwrap();
    assert!(!model.pins_current(GENERATION));
    // The backend lists pins oldest pin first.
    model
        .pins(&ticket(&model), &[message(1), message(2), message(0)])
        .unwrap();
    assert!(model.pins_current(GENERATION));
    assert!(!model.pins_current(GENERATION + 1));
    assert_eq!(model.pin_order(), [id(0), id(2), id(1)]);
    model.event(&pin(4, true), GENERATION).unwrap();
    model.event(&pin(2, false), GENERATION).unwrap();
    assert_eq!(model.pin_order(), [id(4), id(0), id(1)]);
}

#[test]
fn a_pin_list_fetched_before_a_pin_event_does_not_undo_it() {
    let mut model = Model::default();
    model
        .merge_page(&ticket(&model), &[message(0)], false)
        .unwrap();
    let before = ticket(&model);
    model.event(&pin(0, true), GENERATION).unwrap();
    model.pins(&before, &[]).unwrap();
    assert_eq!(model.pin_order(), [id(0)]);
    let before = ticket(&model);
    model.event(&pin(0, false), GENERATION).unwrap();
    model.pins(&before, &[message(0)]).unwrap();
    assert_eq!(model.pin_order(), Vec::<[u8; 16]>::new());
}

#[test]
fn a_deleted_pin_leaves_the_list() {
    let mut model = Model::default();
    model
        .pins(&ticket(&model), &[message(0), message(1)])
        .unwrap();
    model
        .event(
            &KnownEvent::MessageDelete(Box::new(MessageDeleteEvent {
                id: Uuid::from_u128(2).to_string(),
                channel_id: Uuid::from_u128(CHANNEL).to_string(),
            })),
            GENERATION,
        )
        .unwrap();
    assert_eq!(model.pin_order(), [id(0)]);
}
