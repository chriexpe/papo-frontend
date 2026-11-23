//! The @ query and the people it finds, from synthetic rosters.

use super::*;
use papo_sdk::{Generation, UserSummary};
use serde_json::json;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(0x1000 + n)
}

/// A server channel: not one of the snapshot's direct conversations.
fn channel() -> Uuid {
    id(50)
}

/// A roster of `names`, in order, the first one the account's own; `owner` makes the account
/// the server's owner, who may notify everyone.
fn roster(names: &[&str], owner: bool) -> ResourceSnapshot {
    roster_with_dm(names, owner, None)
}

/// [`roster`], with a direct conversation `dm` with person `peer`.
fn roster_with_dm(names: &[&str], owner: bool, dm: Option<(Uuid, u128)>) -> ResourceSnapshot {
    let members = names
        .iter()
        .zip(0_u128..)
        .map(|(name, n)| UserSummary {
            id: Some(id(n).to_string()),
            username: Some((*name).to_owned()),
            // The account's own row must say its roles.
            roles: Some(Vec::new()),
            ..UserSummary::default()
        })
        .collect();
    let server = serde_json::from_value(json!({
        "id": id(99).to_string(),
        "name": "synthetic",
        "owner_id": if owner { id(0) } else { id(98) }.to_string(),
    }))
    .unwrap();
    ResourceSnapshot::fixture(
        Generation::for_test(1),
        Some(server),
        Vec::new(),
        Vec::new(),
        members,
        Vec::new(),
        dm.map(|(channel, peer)| papo_sdk::DirectConversation {
            id: channel.to_string(),
            user: UserSummary {
                id: Some(id(peer).to_string()),
                ..UserSummary::default()
            },
            created_at: "2026-01-01T00:00:00Z".into(),
            last_message: None,
            last_read_message: None,
            last_read_at: None,
            unread_count: 0,
        })
        .into_iter()
        .collect(),
        Vec::new(),
        id(0),
    )
    .unwrap()
}

fn names(found: &[MentionSuggestion]) -> Vec<&str> {
    found.iter().map(|found| found.name.as_str()).collect()
}

fn at_end(text: &str) -> Option<MentionQuery> {
    mention_query(text, text.len(), &[])
}

#[test]
fn an_at_starting_a_word_opens_a_query() {
    assert_eq!(
        at_end("oi @ma"),
        Some(MentionQuery {
            byte_range: 3..6,
            text: "ma".to_owned()
        })
    );
    assert_eq!(at_end("@").map(|query| query.text), Some(String::new()));
}

#[test]
fn an_address_a_space_or_a_mention_already_made_is_no_query() {
    assert_eq!(at_end("ana@example"), None);
    assert_eq!(at_end("@ma rina"), None);
    assert_eq!(
        mention_query("@marina", 7, std::slice::from_ref(&(0..7))),
        None
    );
    assert_eq!(at_end(&format!("@{}", "a".repeat(33))), None);
}

#[test]
fn names_starting_with_the_query_come_first() {
    let snapshot = roster(&["ana", "mariana", "marina"], false);
    assert_eq!(
        names(&mention_suggestions(&snapshot, id(0), channel(), "MAR")),
        ["mariana", "marina"]
    );
    assert_eq!(
        names(&mention_suggestions(&snapshot, id(0), channel(), "ana")),
        ["ana", "mariana"],
        "then names containing it"
    );
}

#[test]
fn everyone_leads_only_where_allowed() {
    let owned = roster(&["eva"], true);
    let found = mention_suggestions(&owned, id(0), channel(), "e");
    assert_eq!(found[0].target, MentionTarget::Everyone);
    assert_eq!(names(&found), ["everyone", "eva"]);
    let member = roster(&["eva"], false);
    assert_eq!(
        names(&mention_suggestions(&member, id(0), channel(), "e")),
        ["eva"]
    );
}

#[test]
fn an_empty_nickname_falls_back_to_the_username() {
    assert_eq!(display_name(Some(""), Some("ana")), Some("ana"));
    assert_eq!(display_name(Some("Ana"), Some("ana")), Some("Ana"));
    assert_eq!(display_name(Some(""), None), None);
}

#[test]
fn a_direct_conversation_offers_no_mentions() {
    let dm = id(60);
    // The owner could mention everyone in a server channel.
    let snapshot = roster_with_dm(&["me", "eva", "ana"], true, Some((dm, 1)));
    assert_eq!(
        names(&mention_suggestions(&snapshot, id(0), dm, "")),
        [""; 0]
    );
    assert_eq!(
        names(&mention_suggestions(&snapshot, id(0), dm, "ev")),
        [""; 0]
    );
    assert!(
        names(&mention_suggestions(&snapshot, id(0), channel(), "e")).contains(&"everyone"),
        "the same account still mentions in a channel"
    );
}
