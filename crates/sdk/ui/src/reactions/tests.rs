//! Chips, quick reactions and the reactions list from synthetic messages, queues and pages.

use std::sync::Arc;

use papo_sdk::{
    MessageTarget, Operation,
    operations::interactions::{MessageReactionSummary, MessageReactionUser, MessageUserReaction},
};

use super::*;

const ME: u128 = 1;
const MESSAGE: u128 = 50;

fn unicode(value: &str) -> ReactionEmoji {
    ReactionEmoji::Unicode(value.into())
}

fn message(totals: &[(&str, i64)], mine: &[&str]) -> Message {
    Message {
        id: Some(Uuid::from_u128(MESSAGE).to_string()),
        reactions: Some(
            totals
                .iter()
                .map(|(unicode, count)| MessageReactionSummary {
                    emoji_id: None,
                    unicode: Some((*unicode).into()),
                    count: Some(*count),
                })
                .collect(),
        ),
        user_reactions: Some(
            mine.iter()
                .map(|unicode| MessageUserReaction {
                    id: None,
                    emoji_id: None,
                    unicode: Some((*unicode).into()),
                })
                .collect(),
        ),
        ..Message::default()
    }
}

fn outgoing(operations: Vec<(OperationState, &str)>) -> OutgoingSnapshot {
    OutgoingSnapshot {
        account: None,
        version: 0,
        upload_progress: None,
        fault: None,
        operations: operations
            .into_iter()
            .enumerate()
            .map(|(index, (state, unicode))| {
                Arc::new(Operation {
                    id: Uuid::from_u128(900 + index as u128),
                    channel_id: Uuid::from_u128(100),
                    kind: Arc::new(OperationKind::Reaction {
                        target: MessageTarget::Remote(Uuid::from_u128(MESSAGE)),
                        emoji_id: None,
                        unicode: Some(unicode.into()),
                        add: true,
                    }),
                    state,
                    attempts: 0,
                    depends_on: None,
                    failure: None,
                    message_id: None,
                    history_projected: false,
                    message: None,
                    created_at_us: 0,
                })
            })
            .collect(),
    }
}

fn page(reactions: &[(u128, &str, u32)], more: bool) -> ReactorPage {
    let mut groups: Vec<MessageReactionGroup> = Vec::new();
    for (user, unicode, second) in reactions {
        let entry = MessageReactionUser {
            id: Some(Uuid::from_u128(u128::from(*second) + 10_000).to_string()),
            user_id: Some(Uuid::from_u128(*user).to_string()),
            created_at: Some(format!("2026-01-01T00:00:{second:02}.000000+00:00")),
        };
        match groups
            .iter_mut()
            .find(|group| group.unicode.as_deref() == Some(*unicode))
        {
            Some(group) => group.users.get_or_insert_with(Vec::new).push(entry),
            None => groups.push(MessageReactionGroup {
                emoji_id: None,
                unicode: Some((*unicode).into()),
                count: None,
                users: Some(vec![entry]),
            }),
        }
    }
    ReactorPage::for_test(groups, more)
}

#[test]
fn chips_follow_the_backend_and_mark_yours_and_what_waits() {
    let held = message(&[("🧀", 4), ("👍", 3), ("😂", 0)], &["👍"]);
    let chips = reaction_chips(&held, &outgoing(vec![(OperationState::Queued, "🧀")]));
    assert_eq!(
        chips,
        [
            ReactionChip {
                emoji: unicode("🧀"),
                count: 4,
                mine: false,
                waiting: true,
            },
            ReactionChip {
                emoji: unicode("👍"),
                count: 3,
                mine: true,
                waiting: false,
            },
        ]
    );
}

#[test]
fn a_refused_or_confirmed_change_does_not_wait() {
    let held = message(&[("🧀", 4)], &[]);
    let queue = outgoing(vec![
        (OperationState::Failed, "🧀"),
        (OperationState::Unknown, "🧀"),
        (OperationState::Sent, "🧀"),
    ]);
    assert!(!reaction_chips(&held, &queue)[0].waiting);
}

#[test]
fn a_chip_toggles_only_with_permission_and_not_while_waiting() {
    let chip = ReactionChip {
        emoji: unicode("👍"),
        count: 1,
        mine: true,
        waiting: false,
    };
    let reader = ChannelCapabilities {
        remove_reaction: true,
        ..ChannelCapabilities::default()
    };
    assert!(chip.can_toggle(Some(reader)));
    assert!(!chip.can_toggle(None));
    let theirs = ReactionChip {
        mine: false,
        ..chip.clone()
    };
    assert!(!theirs.can_toggle(Some(reader)));
    let waiting = ReactionChip {
        waiting: true,
        ..chip
    };
    assert!(!waiting.can_toggle(Some(reader)));
}

#[test]
fn quick_reactions_rank_your_use_then_fill_from_the_defaults() {
    let usage = vec![
        ("🧀".to_owned(), 9),
        (Uuid::from_u128(7).to_string(), 8),
        ("👍".to_owned(), 3),
    ];
    assert_eq!(quick_reactions(&usage, 3), ["🧀", "👍", "😂"]);
    assert_eq!(
        quick_reactions(&usage, 8),
        ["🧀", "👍", "😂", "❤️", "🔥", "😭", "👀", "🙏"]
    );
    assert_eq!(quick_reactions(&[], 8), DEFAULT_QUICK_REACTIONS);
}

#[test]
fn the_list_puts_you_first_then_everyone_in_the_order_they_reacted() {
    let held = message(&[("👍", 3), ("🧀", 2)], &["👍"]);
    let chips = reaction_chips(&held, &outgoing(Vec::new()));
    let mut list = Reactors::new(Uuid::from_u128(ME), &chips);
    // Groups list one emoji each; the order across them comes from the reaction times.
    list.push(&page(
        &[
            (ME, "👍", 1),
            (3, "👍", 4),
            (2, "🧀", 2),
            (3, "🧀", 3),
            (2, "👍", 5),
        ],
        false,
    ));
    let all = list.rows(None);
    let users: Vec<_> = all.iter().map(|reactor| reactor.user).collect();
    assert_eq!(
        users,
        [Uuid::from_u128(ME), Uuid::from_u128(2), Uuid::from_u128(3)]
    );
    assert!(all[0].you);
    assert_eq!(all[1].emoji, [unicode("🧀"), unicode("👍")]);
    assert_eq!(all[2].emoji, [unicode("🧀"), unicode("👍")]);
    let cheese: Vec<_> = list
        .rows(Some(&unicode("🧀")))
        .iter()
        .map(|r| r.user)
        .collect();
    assert_eq!(cheese, [Uuid::from_u128(2), Uuid::from_u128(3)]);
    assert!(!list.wants_more(None, &chips));
}

#[test]
fn a_filtered_tab_keeps_paging_until_it_holds_its_count() {
    let held = message(&[("👍", 2), ("🧀", 1)], &[]);
    let chips = reaction_chips(&held, &outgoing(Vec::new()));
    let mut list = Reactors::new(Uuid::from_u128(ME), &chips);
    list.push(&page(&[(2, "👍", 1), (3, "👍", 2)], true));
    let cheese = unicode("🧀");
    assert!(!list.wants_more(Some(&unicode("👍")), &chips));
    assert!(list.wants_more(Some(&cheese), &chips));
    assert!(list.wants_more(None, &chips));
    list.push(&page(&[(4, "🧀", 3)], false));
    assert!(!list.wants_more(Some(&cheese), &chips));
    assert!(list.exhausted());
}
