//! What Ctrl+K finds, from a synthetic server.

use super::*;
use papo_sdk::{Channel, DirectConversation, Generation, ServerAddress, UserSummary};
use serde_json::json;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(0x4000 + n)
}

fn account() -> AccountId {
    AccountId::new(
        ServerAddress::parse("https://lumen.example.invalid")
            .unwrap()
            .id(),
        id(0),
    )
}

fn person(n: u128, username: &str, nickname: &str) -> UserSummary {
    UserSummary {
        id: Some(id(n).to_string()),
        username: Some(username.into()),
        nickname: nickname.into(),
        roles: Some(Vec::new()),
        ..UserSummary::default()
    }
}

fn channel(n: u128, name: &str, kind: &str) -> Channel {
    Channel {
        id: Some(id(n).to_string()),
        name: Some(name.into()),
        r#type: Some(kind.into()),
        permissions: Some(vec![]),
        ..Channel::default()
    }
}

/// Lumen: #general, #design, a voice room "devs"; you, Dev (a DM), Devi;
/// Gene blocked.
fn lumen() -> ResourceSnapshot {
    let server = serde_json::from_value(json!({
        "id": id(99).to_string(),
        "name": "Lumen Devs",
        "owner_id": id(98).to_string(),
    }))
    .unwrap();
    ResourceSnapshot::fixture(
        Generation::for_test(1),
        Some(server),
        vec![
            channel(20, "general", "text"),
            channel(21, "design", "text"),
            channel(22, "devs", "voice"),
        ],
        Vec::new(),
        vec![
            person(0, "me", ""),
            person(1, "dev", "Dev"),
            person(2, "devi", "Devi"),
            person(3, "gene", "Gene"),
        ],
        Vec::new(),
        vec![DirectConversation {
            id: id(50).to_string(),
            user: person(1, "dev", "Dev"),
            created_at: "2026-01-01T00:00:00Z".into(),
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            unread_count: 0,
        }],
        vec![person(3, "gene", "Gene")],
        id(0),
    )
    .unwrap()
}

fn found(query: &str) -> Vec<(String, JumpKind)> {
    let snapshot = lumen();
    jump_targets(&[(account(), &snapshot)], query)
        .into_iter()
        .map(|target| (target.name, target.kind))
        .collect()
}

fn names(query: &str) -> Vec<String> {
    found(query).into_iter().map(|(name, _)| name).collect()
}

#[test]
fn one_ranked_list_across_kinds_exact_and_starting_first() {
    // Dev is exact (a DM); then names starting with "dev"; the server has
    // a word starting with it; a voice room is no target.
    assert_eq!(names("dev"), ["Dev", "Devi", "Lumen Devs"]);
    let kinds: Vec<_> = found("dev").into_iter().map(|(_, kind)| kind).collect();
    assert_eq!(
        kinds,
        [
            JumpKind::Direct {
                channel: id(50),
                peer: id(1)
            },
            JumpKind::Person { user: id(2) },
            JumpKind::Server
        ]
    );
}

#[test]
fn a_leading_character_narrows_to_one_kind() {
    assert_eq!(names("#de"), ["design"]);
    assert_eq!(
        names("#"),
        ["design", "general"],
        "a prefix alone lists its kind"
    );
    assert_eq!(names("@de"), ["Dev", "Devi"]);
    assert_eq!(names("*lu"), ["Lumen Devs"]);
}

#[test]
fn you_people_you_block_and_an_empty_field_find_nothing() {
    let you = JumpKind::Person { user: id(0) };
    assert!(
        found("me").iter().all(|(_, kind)| *kind != you),
        "never you"
    );
    assert_eq!(names("gene"), ["general"], "Gene is blocked");
    assert_eq!(names("  "), [""; 0]);
}

#[test]
fn banned_members_are_no_target_and_several_accounts_each_bring_theirs() {
    let mut people = vec![person(0, "me", ""), person(5, "zé", "Zé")];
    people.push(UserSummary {
        banned: Some(true),
        ..person(6, "zeca", "Zeca")
    });
    let other = ResourceSnapshot::fixture(
        Generation::for_test(1),
        None,
        Vec::new(),
        Vec::new(),
        people,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        id(0),
    )
    .unwrap();
    let second = AccountId::new(
        ServerAddress::parse("https://orla.example.invalid")
            .unwrap()
            .id(),
        id(0),
    );
    let lumen = lumen();
    let found = jump_targets(&[(account(), &lumen), (second, &other)], "z");
    let names: Vec<_> = found.iter().map(|target| target.name.as_str()).collect();
    assert_eq!(
        names,
        ["Zé"],
        "a non-ASCII name matches; the banned one does not"
    );
    assert_eq!(found[0].account, second);
}

#[test]
fn at_most_fifty_targets_the_best_first() {
    let channels: Vec<_> = (0..80)
        .map(|n| channel(100 + n, &format!("room-{n:02}"), "text"))
        .chain([channel(300, "room", "text")])
        .collect();
    let many = ResourceSnapshot::fixture(
        Generation::for_test(1),
        None,
        channels,
        Vec::new(),
        vec![person(0, "me", "")],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        id(0),
    )
    .unwrap();
    let found = jump_targets(&[(account(), &many)], "room");
    assert_eq!(found.len(), MAX_JUMP_TARGETS);
    assert_eq!(found[0].name, "room", "the exact name first");
    assert_eq!(found[1].name, "room-00");
}
