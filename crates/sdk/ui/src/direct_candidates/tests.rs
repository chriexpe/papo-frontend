//! The New message picker's people, from synthetic rosters on two servers.

use super::*;
use papo_sdk::{Generation, ServerAddress, UserSummary};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(0x3000 + n)
}

fn account(server: &str) -> AccountId {
    AccountId::new(
        ServerAddress::parse(&format!("https://{server}.example.invalid"))
            .unwrap()
            .id(),
        id(0),
    )
}

fn person(n: u128, username: &str, nickname: &str, banned: bool) -> UserSummary {
    UserSummary {
        id: Some(id(n).to_string()),
        username: Some(username.into()),
        nickname: nickname.into(),
        banned: Some(banned),
        roles: Some(Vec::new()),
        ..UserSummary::default()
    }
}

fn roster(people: Vec<UserSummary>, blocks: Vec<UserSummary>) -> ResourceSnapshot {
    ResourceSnapshot::fixture(
        Generation::for_test(1),
        None,
        Vec::new(),
        Vec::new(),
        people,
        Vec::new(),
        Vec::new(),
        blocks,
        id(0),
    )
    .unwrap()
}

fn names(found: &[DirectCandidate]) -> Vec<&str> {
    found.iter().map(|c| c.name.as_str()).collect()
}

#[test]
fn everyone_a_to_z_one_row_per_server_without_you_banned_or_blocked() {
    let lumen = roster(
        vec![
            person(0, "me", "", false),
            person(1, "lia", "Lia", false),
            person(2, "bruno", "", false),
            person(3, "spam", "", true),
        ],
        Vec::new(),
    );
    let asilo = roster(
        vec![
            person(0, "me", "", false),
            person(1, "lia", "Lia", false),
            person(4, "ana", "Ana", false),
            person(5, "troll", "", false),
        ],
        vec![person(5, "troll", "", false)],
    );
    let (a, b) = (account("lumen"), account("asilo"));
    let found = direct_candidates(&[(a, &lumen), (b, &asilo)], "");
    assert_eq!(names(&found), ["Ana", "bruno", "Lia", "Lia"]);
    assert_eq!(
        (found[2].account, found[3].account),
        (a, b),
        "in the servers' order"
    );
}

#[test]
fn names_starting_with_the_query_come_before_names_containing_it() {
    let lumen = roster(
        vec![
            person(0, "me", "", false),
            person(1, "lia", "Lia", false),
            person(2, "amelia", "Amélia", false),
            person(3, "rui", "", false),
        ],
        Vec::new(),
    );
    let found = direct_candidates(&[(account("lumen"), &lumen)], "@LI");
    assert_eq!(names(&found), ["Lia", "Amélia"]);
}
